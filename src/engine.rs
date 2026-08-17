use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use bitcoin::secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use bitcoin::{Address, Network, PrivateKey};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

use crate::config::{self, Config};
use crate::db::{self, Db};
use crate::hit;
use crate::notify::Notifier;
use crate::pending;
use crate::status::{self, Status};

/// Supervisor should run `data update` and restart when the engine exits with this code.
pub const RELOAD_EXIT: u8 = 75;

const BATCH: usize = 512;
const REPORT_BLOCK: u64 = 1 << 17;

mod ec {
    use std::ffi::c_void;

    extern "C" {
        fn ec_walk_new(cap: usize) -> *mut c_void;
        fn ec_walk_set_start(w: *mut c_void, pubkey: *const u8, len: usize) -> i32;
        fn ec_walk_batch(w: *mut c_void, n: usize, out_comp: *mut u8, out_uncomp: *mut u8);
        fn ec_walk_free(w: *mut c_void);
    }

    pub struct Walk {
        raw: *mut c_void,
        cap: usize,
    }

    unsafe impl Send for Walk {}

    impl Walk {
        pub fn new(cap: usize) -> Self {
            let raw = unsafe { ec_walk_new(cap) };
            assert!(!raw.is_null(), "ec_walk_new: allocation failed");
            Walk { raw, cap }
        }

        pub fn set_start(&mut self, pubkey: &[u8]) -> bool {
            unsafe { ec_walk_set_start(self.raw, pubkey.as_ptr(), pubkey.len()) == 1 }
        }

        pub fn batch(&mut self, n: usize, comp: &mut [u8], uncomp: Option<&mut [u8]>) {
            assert!(n <= self.cap, "batch {n} exceeds capacity {}", self.cap);
            assert!(comp.len() >= n * 33, "compressed buffer too small");
            let up = match uncomp {
                Some(u) => {
                    assert!(u.len() >= n * 65, "uncompressed buffer too small");
                    u.as_mut_ptr()
                }
                None => std::ptr::null_mut(),
            };
            unsafe { ec_walk_batch(self.raw, n, comp.as_mut_ptr(), up) };
        }
    }

    impl Drop for Walk {
        fn drop(&mut self) {
            unsafe { ec_walk_free(self.raw) };
        }
    }
}

#[inline(always)]
pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let ripe = Ripemd160::digest(sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&ripe);
    out
}

#[cfg(any(neon_hash, x86_hash))]
extern "C" {
    fn hash160_many(pubkeys: *const u8, out20: *mut u8, n: usize);
    fn hash160_many_uncomp(pubkeys: *const u8, out20: *mut u8, n: usize);
}

pub fn simd_name() -> &'static str {
    if !simd_available() {
        return "crate";
    }
    #[cfg(neon_hash)]
    {
        "neon"
    }
    #[cfg(x86_hash)]
    {
        "sha-ni"
    }
    #[cfg(not(any(neon_hash, x86_hash)))]
    {
        "crate"
    }
}

fn simd_available() -> bool {
    #[cfg(neon_hash)]
    {
        true
    }
    #[cfg(x86_hash)]
    {
        std::is_x86_feature_detected!("sha") && std::is_x86_feature_detected!("sse4.1")
    }
    #[cfg(not(any(neon_hash, x86_hash)))]
    {
        false
    }
}

fn hash_batch(comp: &[u8], out: &mut [u8], n: usize) {
    debug_assert!(comp.len() >= n * 33 && out.len() >= n * 20);
    if try_simd(comp, out, n, false) {
        return;
    }
    for i in 0..n {
        let h = hash160(&comp[i * 33..i * 33 + 33]);
        out[i * 20..i * 20 + 20].copy_from_slice(&h);
    }
}

fn hash_batch_uncomp(uncomp: &[u8], out: &mut [u8], n: usize) {
    debug_assert!(uncomp.len() >= n * 65 && out.len() >= n * 20);
    if try_simd(uncomp, out, n, true) {
        return;
    }
    for i in 0..n {
        let h = hash160(&uncomp[i * 65..i * 65 + 65]);
        out[i * 20..i * 20 + 20].copy_from_slice(&h);
    }
}

fn try_simd(data: &[u8], out: &mut [u8], n: usize, uncompressed: bool) -> bool {
    #[cfg(not(any(neon_hash, x86_hash)))]
    {
        let _ = (data, out, n, uncompressed);
        false
    }
    #[cfg(any(neon_hash, x86_hash))]
    {
        if !simd_available() {
            return false;
        }
        unsafe {
            if uncompressed {
                hash160_many_uncomp(data.as_ptr(), out.as_mut_ptr(), n);
            } else {
                hash160_many(data.as_ptr(), out.as_mut_ptr(), n);
            }
        }
        true
    }
}

struct Shared {
    db: Arc<Db>,
    secp: Arc<Secp256k1<All>>,
    keys: AtomicU64,
    hits: AtomicU64,
    running: AtomicBool,
    reload: AtomicBool,
    check_uncompressed: bool,
    walk_span: u64,
    cpu_percent: u8,
    findings: PathBuf,
    data_dir: PathBuf,
}

struct HitNotice {
    address: String,
    compressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Stopped,
    Reload,
}

pub fn run(cfg: &Config) -> Result<RunOutcome, String> {
    let halt = Arc::new(AtomicBool::new(false));
    {
        let halt = Arc::clone(&halt);
        ctrlc::set_handler(move || {
            halt.store(true, Ordering::Relaxed);
        })
        .map_err(|e| e.to_string())?;
    }

    loop {
        if halt.load(Ordering::Relaxed) {
            return Ok(RunOutcome::Stopped);
        }

        let loaded = db::load(cfg).map_err(|e| e.to_string())?;
        println!(
            "Loaded {} unique funded hash160s (P2PKH + P2WPKH) in {:.2?} from {} via {} (~{}MB RAM, {} skipped)",
            loaded.db.len(),
            loaded.elapsed,
            loaded.source,
            loaded.db.lookup_name(),
            loaded.db.ram_bytes() / (1024 * 1024),
            loaded.skipped
        );

        let threads = config::worker_count(cfg);
        println!(
            "Running on {threads} worker thread(s) | uncompressed={} | simd={} | cpu={} | lookup={} | walk_span={}",
            cfg.check_uncompressed,
            simd_name(),
            cfg.cpu_percent,
            loaded.db.lookup_name(),
            cfg.walk_span
        );

        let shared = Arc::new(Shared {
            db: Arc::new(loaded.db),
            secp: Arc::new(Secp256k1::new()),
            keys: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            running: AtomicBool::new(true),
            reload: AtomicBool::new(false),
            check_uncompressed: cfg.check_uncompressed,
            walk_span: cfg.walk_span,
            cpu_percent: cfg.cpu_percent,
            findings: cfg.findings.clone(),
            data_dir: cfg.data_dir.clone(),
        });

        let (tx, rx) = mpsc::channel::<HitNotice>();
        let mut workers = Vec::with_capacity(threads);
        for _ in 0..threads {
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            workers.push(thread::spawn(move || process(&shared, tx)));
        }
        drop(tx);

        let outcome = reporter(cfg, &shared, &halt, rx, threads, &loaded.source);
        shared.running.store(false, Ordering::Relaxed);
        for worker in workers {
            let _ = worker.join();
        }

        match outcome {
            RunOutcome::Stopped => return Ok(RunOutcome::Stopped),
            RunOutcome::Reload => {
                if !cfg.auto_update {
                    return Ok(RunOutcome::Reload);
                }
                println!("releasing snapshot and downloading a fresh funded set");
                drop(shared);
                match db::refresh_snapshot(cfg, &cfg.source_url) {
                    Ok((count, skipped, elapsed)) => {
                        println!(
                            "Updated snapshot to {count} hash160s in {elapsed:.2?} ({skipped} skipped)"
                        );
                    }
                    Err(error) => {
                        eprintln!("snapshot update failed: {error}; reloading previous file");
                        let notifier = Notifier::from_config(&cfg.notify);
                        notifier.send(
                            "Plutus 更新失败",
                            &format!("node={} error={error}", config::node_name()),
                        );
                    }
                }
            }
        }
    }
}

fn process(shared: &Shared, hits: Sender<HitNotice>) {
    let mut rng = rand::thread_rng();
    let mut walk = ec::Walk::new(BATCH);
    let mut comp = vec![0u8; BATCH * 33];
    let mut h160 = vec![0u8; BATCH * 20];
    let mut uncomp = if shared.check_uncompressed {
        vec![0u8; BATCH * 65]
    } else {
        Vec::new()
    };
    let mut h160_u = if shared.check_uncompressed {
        vec![0u8; BATCH * 20]
    } else {
        Vec::new()
    };
    let mut since_report: u64 = 0;

    while shared.running.load(Ordering::Relaxed) {
        let start_secret = random_secret(&mut rng);
        let start_pub = PublicKey::from_secret_key(&shared.secp, &start_secret);
        if !walk.set_start(&start_pub.serialize()) {
            continue;
        }

        let mut base: u64 = 0;
        while base < shared.walk_span && shared.running.load(Ordering::Relaxed) {
            let batch_started = Instant::now();
            if shared.check_uncompressed {
                walk.batch(BATCH, &mut comp, Some(&mut uncomp));
            } else {
                walk.batch(BATCH, &mut comp, None);
            }

            hash_batch(&comp, &mut h160, BATCH);
            if shared.check_uncompressed {
                hash_batch_uncomp(&uncomp, &mut h160_u, BATCH);
            }

            for i in 0..BATCH {
                let hash: &[u8; 20] = h160[i * 20..i * 20 + 20].try_into().unwrap();
                if shared.db.contains(hash) {
                    on_hit(shared, &start_secret, base + i as u64, true, &hits);
                }
                if shared.check_uncompressed {
                    let hash_u: &[u8; 20] = h160_u[i * 20..i * 20 + 20].try_into().unwrap();
                    if shared.db.contains(hash_u) {
                        on_hit(shared, &start_secret, base + i as u64, false, &hits);
                    }
                }
            }

            base += BATCH as u64;
            since_report += BATCH as u64;
            if since_report >= REPORT_BLOCK {
                shared.keys.fetch_add(since_report, Ordering::Relaxed);
                since_report = 0;
            }
            throttle(shared.cpu_percent, batch_started);
        }
    }
    if since_report > 0 {
        shared.keys.fetch_add(since_report, Ordering::Relaxed);
    }
}

fn on_hit(
    shared: &Shared,
    start_secret: &SecretKey,
    offset: u64,
    compressed: bool,
    hits: &Sender<HitNotice>,
) {
    let address = match persist_hit(
        &shared.secp,
        start_secret,
        offset,
        compressed,
        &shared.findings,
    ) {
        Ok(address) => address,
        Err(error) => {
            eprintln!("failed to persist hit: {error}");
            return;
        }
    };
    shared.hits.fetch_add(1, Ordering::Relaxed);
    if let Err(error) =
        pending::enqueue(&shared.data_dir, &address, compressed, pending::unix_now())
    {
        eprintln!("failed to persist pending hit alert: {error}");
    }
    let _ = hits.send(HitNotice {
        address,
        compressed,
    });
}

fn persist_hit(
    secp: &Secp256k1<All>,
    start_secret: &SecretKey,
    offset: u64,
    compressed: bool,
    findings: &std::path::Path,
) -> Result<String, String> {
    let mut tweak = [0u8; 32];
    tweak[24..].copy_from_slice(&offset.to_be_bytes());
    let secret_key = start_secret
        .add_tweak(&Scalar::from_be_bytes(tweak).expect("offset < order"))
        .expect("valid secret");

    let mut private_key = PrivateKey::new(secret_key, Network::Bitcoin);
    private_key.compressed = compressed;
    let public_key = bitcoin::PublicKey::from_private_key(secp, &private_key);
    let address = Address::p2pkh(&public_key, Network::Bitcoin);
    let address_s = address.to_string();

    let record = format!(
        "{}\n{}\n{}\n{}\n",
        secret_key.display_secret(),
        private_key.to_wif(),
        public_key,
        address_s
    );
    println!("!!! MATCH FOUND -> {address_s}");
    hit::persist(findings, &record).map_err(|e| e.to_string())?;
    Ok(address_s)
}

fn throttle(cpu_percent: u8, started: Instant) {
    if cpu_percent >= 100 {
        return;
    }
    let work = started.elapsed();
    let sleep = work.mul_f64(f64::from(100 - cpu_percent) / f64::from(cpu_percent.max(1)));
    if sleep > Duration::from_micros(50) {
        thread::sleep(sleep);
    }
}

fn random_secret(rng: &mut impl rand::RngCore) -> SecretKey {
    loop {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        if let Ok(sk) = SecretKey::from_slice(&bytes) {
            return sk;
        }
    }
}

fn reporter(
    cfg: &Config,
    shared: &Shared,
    halt: &AtomicBool,
    rx: Receiver<HitNotice>,
    threads: usize,
    source: &str,
) -> RunOutcome {
    let notifier = Notifier::from_config(&cfg.notify);
    let node = config::node_name();
    notifier.send(
        "Plutus 已启动",
        &format!(
            "node={node} threads={threads} db={} ram_mb={} uncompressed={} simd={} cpu={} lookup={} snapshot={source}",
            shared.db.len(),
            shared.db.ram_bytes() / (1024 * 1024),
            cfg.check_uncompressed,
            simd_name(),
            cfg.cpu_percent,
            shared.db.lookup_name(),
        ),
    );

    let start = Instant::now();
    let started_unix = status::unix_now();
    let mut last_total = 0u64;
    let mut last_at = start;
    let mut last_heartbeat = Instant::now();
    let mut last_age_check = Instant::now();
    let mut last_progress_log = Instant::now();
    let heartbeat = Duration::from_secs(cfg.heartbeat_minutes.saturating_mul(60).max(60));
    let progress_log = Duration::from_secs(3600);
    flush_pending(cfg, &notifier);

    while shared.running.load(Ordering::Relaxed) {
        if halt.load(Ordering::Relaxed) {
            shared.running.store(false, Ordering::Relaxed);
            break;
        }
        thread::sleep(Duration::from_secs(3));
        drain_hits(&rx, cfg, &notifier);

        let now = Instant::now();
        let total = shared.keys.load(Ordering::Relaxed);
        let hits = shared.hits.load(Ordering::Relaxed);
        let dt = (now - last_at).as_secs_f64().max(0.001);
        let inst = (total.saturating_sub(last_total)) as f64 / dt;
        let avg = total as f64 / now.duration_since(start).as_secs_f64().max(0.001);
        if last_progress_log.elapsed() >= progress_log {
            println!("still running | checked {total} keys | {avg:.0} keys/s avg | hits {hits}");
            last_progress_log = now;
        }

        let snapshot_age_hours = db::snapshot_age_secs(&cfg.snapshot)
            .map(|s| s as f64 / 3600.0)
            .unwrap_or(0.0);

        let snapshot = Status {
            started_unix,
            now_unix: status::unix_now(),
            uptime_secs: now.duration_since(start).as_secs(),
            keys_checked: total,
            keys_per_sec_avg: avg,
            keys_per_sec_inst: inst,
            hits,
            db_size: shared.db.len(),
            threads,
            check_uncompressed: cfg.check_uncompressed,
            simd: simd_name(),
            snapshot: source.to_owned(),
            lookup: shared.db.lookup_name().to_owned(),
            ram_bytes: shared.db.ram_bytes(),
            cpu_percent: cfg.cpu_percent,
            node: node.clone(),
            running: true,
            reload_requested: false,
        };
        if let Err(error) = status::write_atomic(&cfg.status, &snapshot) {
            eprintln!("status write failed: {error}");
        }

        if last_heartbeat.elapsed() >= heartbeat {
            notifier.send(
                "Plutus 还活着",
                &format!(
                    "node={node} keys={total} avg_keys_s={avg:.0} hits={hits} db={} uptime_h={:.1} snapshot_age_h={snapshot_age_hours:.1} cpu={}",
                    shared.db.len(),
                    snapshot.uptime_secs as f64 / 3600.0,
                    cfg.cpu_percent
                ),
            );
            last_heartbeat = Instant::now();
        }

        if cfg.auto_update && last_age_check.elapsed() >= Duration::from_secs(600) {
            last_age_check = Instant::now();
            let max_age = cfg.max_snapshot_age_hours.saturating_mul(3600);
            if db::snapshot_age_secs(&cfg.snapshot).unwrap_or(0) >= max_age {
                println!(
                    "snapshot older than {}h, recycling workers to refresh",
                    cfg.max_snapshot_age_hours
                );
                notifier.send(
                    "Plutus 正在更新地址库",
                    &format!("node={node} snapshot_age_h={snapshot_age_hours:.1}"),
                );
                shared.reload.store(true, Ordering::Relaxed);
                shared.running.store(false, Ordering::Relaxed);
                break;
            }
        }

        last_total = total;
        last_at = now;
    }

    drain_hits(&rx, cfg, &notifier);
    let total = shared.keys.load(Ordering::Relaxed);
    let hits = shared.hits.load(Ordering::Relaxed);
    let reload = shared.reload.load(Ordering::Relaxed) && !halt.load(Ordering::Relaxed);
    println!("shutting down | checked {total} keys | hits {hits} | reload={reload}");
    if !reload {
        notifier.send(
            "Plutus 已停止",
            &format!(
                "node={node} keys={total} hits={hits} db={}",
                shared.db.len()
            ),
        );
    }
    let snapshot = Status {
        started_unix,
        now_unix: status::unix_now(),
        uptime_secs: start.elapsed().as_secs(),
        keys_checked: total,
        keys_per_sec_avg: 0.0,
        keys_per_sec_inst: 0.0,
        hits,
        db_size: shared.db.len(),
        threads,
        check_uncompressed: cfg.check_uncompressed,
        simd: simd_name(),
        snapshot: source.to_owned(),
        lookup: shared.db.lookup_name().to_owned(),
        ram_bytes: shared.db.ram_bytes(),
        cpu_percent: cfg.cpu_percent,
        node,
        running: false,
        reload_requested: reload,
    };
    let _ = status::write_atomic(&cfg.status, &snapshot);
    if reload {
        RunOutcome::Reload
    } else {
        RunOutcome::Stopped
    }
}

fn drain_hits(rx: &Receiver<HitNotice>, cfg: &Config, notifier: &Notifier) {
    let now = pending::unix_now();
    while let Ok(hit) = rx.try_recv() {
        if let Err(error) = pending::enqueue(&cfg.data_dir, &hit.address, hit.compressed, now) {
            eprintln!("failed to persist pending hit alert: {error}");
        }
    }
    flush_pending(cfg, notifier);
}

fn flush_pending(cfg: &Config, notifier: &Notifier) {
    if !notifier.enabled() {
        return;
    }
    let now = pending::unix_now();
    let interval = cfg.notify.hit_repeat_secs;
    let max = cfg.notify.hit_repeat_max;
    let due = match pending::due(&cfg.data_dir, now, interval, max) {
        Ok(items) => items,
        Err(error) => {
            eprintln!("pending hit queue: {error}");
            return;
        }
    };
    for item in due {
        let enc = if item.compressed {
            "compressed"
        } else {
            "uncompressed"
        };
        let attempt = item.sent.saturating_add(1);
        let title = if max == 0 {
            format!("Plutus 命中 #{attempt}")
        } else {
            format!("Plutus 命中 {attempt}/{max}")
        };
        let body = format!("address={} encoding={enc} attempt={attempt}", item.address);
        match notifier.send_hit(&title, &body, attempt) {
            Ok(()) => {
                if let Err(error) =
                    pending::mark_sent(&cfg.data_dir, &item.address, item.compressed, now)
                {
                    eprintln!("failed to record hit alert: {error}");
                }
            }
            Err(error) => eprintln!("notify failed: {error}"),
        }
    }
}

#[cfg(test)]
fn generator(secp: &Secp256k1<All>) -> PublicKey {
    let mut one = [0u8; 32];
    one[31] = 1;
    PublicKey::from_secret_key(secp, &SecretKey::from_slice(&one).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret_from_u8(last: u8) -> SecretKey {
        let mut b = [0u8; 32];
        b[31] = last;
        SecretKey::from_slice(&b).unwrap()
    }

    #[test]
    fn derivation_matches_known_vectors() {
        let secp = Secp256k1::new();
        let sk = secret_from_u8(1);

        let mut pk_c = PrivateKey::new(sk, Network::Bitcoin);
        pk_c.compressed = true;
        let addr_c = Address::p2pkh(
            &bitcoin::PublicKey::from_private_key(&secp, &pk_c),
            Network::Bitcoin,
        );
        assert_eq!(addr_c.to_string(), "1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH");

        let mut pk_u = PrivateKey::new(sk, Network::Bitcoin);
        pk_u.compressed = false;
        let addr_u = Address::p2pkh(
            &bitcoin::PublicKey::from_private_key(&secp, &pk_u),
            Network::Bitcoin,
        );
        assert_eq!(addr_u.to_string(), "1EHNa6Q4Jz2uvNExL497mE43ikXhwF6kZm");
    }

    #[test]
    fn hotloop_hash160_matches_db_decode() {
        let secp = Secp256k1::new();
        let pk = PublicKey::from_secret_key(&secp, &secret_from_u8(1));
        let raw = bitcoin::base58::decode_check("1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH").unwrap();
        assert_eq!(raw[0], 0x00);
        assert_eq!(&hash160(&pk.serialize())[..], &raw[1..21]);
    }

    #[test]
    fn sequential_walk_reconstructs_secret() {
        let secp = Secp256k1::new();
        let g = generator(&secp);
        let start = secret_from_u8(123);
        let mut pk = PublicKey::from_secret_key(&secp, &start);
        for offset in 0..2000u64 {
            let mut tweak = [0u8; 32];
            tweak[24..].copy_from_slice(&offset.to_be_bytes());
            let sk = start
                .add_tweak(&Scalar::from_be_bytes(tweak).unwrap())
                .unwrap();
            assert_eq!(
                pk,
                PublicKey::from_secret_key(&secp, &sk),
                "offset {offset}"
            );
            pk = pk.combine(&g).unwrap();
        }
    }

    #[test]
    fn batch_walk_matches_combine() {
        let secp = Secp256k1::new();
        let g = generator(&secp);
        let start_pub = PublicKey::from_secret_key(&secp, &secret_from_u8(77));

        let n = 4096usize;
        let mut walk = ec::Walk::new(n);
        assert!(walk.set_start(&start_pub.serialize()));
        let mut comp = vec![0u8; n * 33];
        walk.batch(n, &mut comp, None);

        let mut pk = start_pub;
        for i in 0..n {
            assert_eq!(
                &comp[i * 33..i * 33 + 33],
                &pk.serialize()[..],
                "offset {i}"
            );
            pk = pk.combine(&g).unwrap();
        }
    }

    #[test]
    fn batch_walk_continues_across_calls() {
        let secp = Secp256k1::new();
        let g = generator(&secp);
        let start_pub = PublicKey::from_secret_key(&secp, &secret_from_u8(5));

        let n = 300usize;
        let mut walk = ec::Walk::new(n);
        assert!(walk.set_start(&start_pub.serialize()));
        let mut first = vec![0u8; n * 33];
        let mut second = vec![0u8; n * 33];
        walk.batch(n, &mut first, None);
        walk.batch(n, &mut second, None);

        let mut pk = start_pub;
        for _ in 0..n {
            pk = pk.combine(&g).unwrap();
        }
        for i in 0..n {
            assert_eq!(
                &second[i * 33..i * 33 + 33],
                &pk.serialize()[..],
                "offset {}",
                n + i
            );
            pk = pk.combine(&g).unwrap();
        }
    }

    #[test]
    fn simd_batch_hash160_matches_crate() {
        let secp = Secp256k1::new();
        let start_pub = PublicKey::from_secret_key(&secp, &secret_from_u8(7));

        let n = 130usize;
        let mut walk = ec::Walk::new(n);
        assert!(walk.set_start(&start_pub.serialize()));
        let mut comp = vec![0u8; n * 33];
        walk.batch(n, &mut comp, None);

        let mut got = vec![0u8; n * 20];
        hash_batch(&comp, &mut got, n);

        for i in 0..n {
            let want = hash160(&comp[i * 33..i * 33 + 33]);
            assert_eq!(&got[i * 20..i * 20 + 20], &want[..], "key {i}");
        }
    }

    #[test]
    fn simd_uncomp_hash160_matches_crate() {
        let secp = Secp256k1::new();
        let start_pub = PublicKey::from_secret_key(&secp, &secret_from_u8(9));

        let n = 130usize;
        let mut walk = ec::Walk::new(n);
        assert!(walk.set_start(&start_pub.serialize()));
        let mut comp = vec![0u8; n * 33];
        let mut unc = vec![0u8; n * 65];
        walk.batch(n, &mut comp, Some(&mut unc));

        let mut got = vec![0u8; n * 20];
        hash_batch_uncomp(&unc, &mut got, n);

        for i in 0..n {
            let want = hash160(&unc[i * 65..i * 65 + 65]);
            assert_eq!(&got[i * 20..i * 20 + 20], &want[..], "uncomp {i}");
        }
    }

    #[test]
    fn batch_uncompressed_matches_combine_and_db() {
        let secp = Secp256k1::new();
        let g = generator(&secp);
        let start_pub = PublicKey::from_secret_key(&secp, &secret_from_u8(1));

        let n = 16usize;
        let mut walk = ec::Walk::new(n);
        assert!(walk.set_start(&start_pub.serialize()));
        let mut comp = vec![0u8; n * 33];
        let mut unc = vec![0u8; n * 65];
        walk.batch(n, &mut comp, Some(&mut unc));

        let db_raw = bitcoin::base58::decode_check("1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH").unwrap();
        assert_eq!(&hash160(&comp[0..33])[..], &db_raw[1..21]);

        let mut pk = start_pub;
        for i in 0..n {
            assert_eq!(&comp[i * 33..i * 33 + 33], &pk.serialize()[..], "comp {i}");
            assert_eq!(
                &unc[i * 65..i * 65 + 65],
                &pk.serialize_uncompressed()[..],
                "unc {i}"
            );
            pk = pk.combine(&g).unwrap();
        }
    }
}
