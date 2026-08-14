use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs::{self, File};
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bitcoin::Address;
use flate2::read::GzDecoder;

use crate::bloom::Bloom;
use crate::config::{Config, Lookup};

const MAGIC_V1: &[u8; 4] = b"PLH1";
const MAGIC_V2: &[u8; 4] = b"PLH2";
const HEADER_LEN: usize = 64;
const VERSION_V2: u16 = 2;
const N_BUCKETS: usize = 65536;
const BUCKET_INDEX_BYTES: usize = N_BUCKETS * 8;
const CHUNK_RECORDS: usize = 1_048_576; // 20 MiB of hash160s per external-sort chunk

thread_local! {
    static BUCKET_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Fast hasher for hash160 keys. The first 8 bytes are already uniform.
#[derive(Default)]
pub struct Hash160Hasher(u64);

impl Hasher for Hash160Hasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let mut buf = [0u8; 8];
        let n = bytes.len().min(8);
        buf[..n].copy_from_slice(&bytes[..n]);
        self.0 = self.0.rotate_left(5) ^ u64::from_le_bytes(buf);
    }
}

pub type HashDb = HashSet<[u8; 20], BuildHasherDefault<Hash160Hasher>>;

pub struct DiskDb {
    bloom: Bloom,
    buckets: Vec<(u32, u32)>,
    file: File,
    rec_off: u64,
    count: usize,
}

pub enum Db {
    Mmap(DiskDb),
    Hash(HashDb),
    Sorted(Vec<[u8; 20]>),
}

impl Db {
    pub fn contains(&self, hash: &[u8; 20]) -> bool {
        match self {
            Db::Mmap(disk) => disk.contains(hash),
            Db::Hash(set) => set.contains(hash),
            Db::Sorted(rows) => rows.binary_search(hash).is_ok(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Db::Mmap(disk) => disk.count,
            Db::Hash(set) => set.len(),
            Db::Sorted(rows) => rows.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn lookup_name(&self) -> &'static str {
        match self {
            Db::Mmap(_) => "mmap",
            Db::Hash(_) => "hash",
            Db::Sorted(_) => "sorted",
        }
    }

    pub fn ram_bytes(&self) -> usize {
        match self {
            Db::Mmap(disk) => disk.bloom.byte_len() + disk.buckets.len() * 8,
            Db::Hash(_) => self.len() * 40,
            Db::Sorted(rows) => rows.len() * 20,
        }
    }

    fn from_hashes(mut hashes: Vec<[u8; 20]>, lookup: Lookup) -> Self {
        hashes.sort_unstable();
        hashes.dedup();
        match lookup {
            Lookup::Sorted => Db::Sorted(hashes),
            Lookup::Hash => {
                let mut set = HashSet::with_capacity_and_hasher(hashes.len(), Default::default());
                set.extend(hashes);
                Db::Hash(set)
            }
            Lookup::Mmap => {
                panic!("mmap databases must be loaded from a PLH2 snapshot, not from_hashes")
            }
        }
    }
}

impl DiskDb {
    fn contains(&self, hash: &[u8; 20]) -> bool {
        if !self.bloom.maybe_contains(hash) {
            return false;
        }
        let bucket = u16::from_be_bytes([hash[0], hash[1]]) as usize;
        let (off, n) = self.buckets[bucket];
        if n == 0 {
            return false;
        }
        let nbytes = n as usize * 20;
        let offset = self.rec_off + u64::from(off) * 20;
        BUCKET_BUF.with(|slot| {
            let mut buf = slot.borrow_mut();
            buf.resize(nbytes, 0);
            if read_exact_at(&self.file, &mut buf, offset).is_err() {
                return false;
            }
            advise_dontneed(&self.file, offset, nbytes);
            let mut lo = 0usize;
            let mut hi = n as usize;
            while lo < hi {
                let mid = (lo + hi) / 2;
                let rec = &buf[mid * 20..mid * 20 + 20];
                match rec.cmp(hash.as_slice()) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => return true,
                }
            }
            false
        })
    }
}

pub struct LoadReport {
    pub db: Db,
    pub skipped: u64,
    pub source: String,
    pub elapsed: std::time::Duration,
}

/// Decode a funded address to the 20-byte hash160 the hot loop matches against,
/// or `None` if it is a type this generator can never produce.
pub fn address_hash160(addr: &str) -> Option<[u8; 20]> {
    match addr.as_bytes().first() {
        Some(b'1') => {
            let raw = bitcoin::base58::decode_check(addr).ok()?;
            if raw.len() == 21 && raw[0] == 0x00 {
                let mut h = [0u8; 20];
                h.copy_from_slice(&raw[1..21]);
                return Some(h);
            }
            None
        }
        Some(b'b') if addr.starts_with("bc1") => {
            let addr = Address::from_str(addr).ok()?.assume_checked();
            let spk = addr.script_pubkey();
            let b = spk.as_bytes();
            if b.len() == 22 && b[0] == 0x00 && b[1] == 0x14 {
                let mut h = [0u8; 20];
                h.copy_from_slice(&b[2..22]);
                return Some(h);
            }
            None
        }
        _ => None,
    }
}

pub fn load(cfg: &Config) -> io::Result<LoadReport> {
    if cfg.snapshot.is_file() {
        return load_snapshot_with(&cfg.snapshot, cfg.lookup, cfg.bits_per_key);
    }
    if cfg.pickle_dir.is_dir() {
        let mut report = load_pickles(cfg)?;
        if matches!(report.db, Db::Mmap(_)) {
            return Ok(report);
        }
        match write_snapshot(&cfg.snapshot, &report.db, cfg.bits_per_key) {
            Ok(()) => {
                println!(
                    "Wrote binary snapshot {} ({} hash160s, ram~{}MB) for fast restarts",
                    cfg.snapshot.display(),
                    report.db.len(),
                    report.db.ram_bytes() / (1024 * 1024)
                );
                report.source = format!(
                    "{} (migrated from {})",
                    cfg.snapshot.display(),
                    cfg.pickle_dir.display()
                );
            }
            Err(error) => {
                eprintln!(
                    "warning: could not write {}: {error} (will reload pickles next start)",
                    cfg.snapshot.display()
                );
            }
        }
        return Ok(report);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "no snapshot at {} and no pickle dir at {}; run `plutus-rustus data update`",
            cfg.snapshot.display(),
            cfg.pickle_dir.display()
        ),
    ))
}

pub fn load_snapshot(path: &Path) -> io::Result<LoadReport> {
    load_snapshot_with(path, Lookup::Mmap, 16)
}

pub fn load_snapshot_with(
    path: &Path,
    lookup: Lookup,
    bits_per_key: u32,
) -> io::Result<LoadReport> {
    let timer = Instant::now();
    let mut header = [0u8; HEADER_LEN];
    {
        let mut file = File::open(path)?;
        file.read_exact(&mut header)?;
    }
    if header[0..4] == MAGIC_V1[..] {
        println!(
            "converting {} from PLH1 to PLH2 (bloom + bucket index)",
            path.display()
        );
        rewrite_snapshot_as_plh2(path, bits_per_key)?;
    }
    match lookup {
        Lookup::Mmap => {
            let db = load_plh2(path)?;
            if db.bloom.bits_per_key() != bits_per_key {
                println!(
                    "rebuilding {} bloom from {} to {bits_per_key} bits/key",
                    path.display(),
                    db.bloom.bits_per_key()
                );
                drop(db);
                rewrite_snapshot_as_plh2(path, bits_per_key)?;
                let db = load_plh2(path)?;
                return Ok(LoadReport {
                    db: Db::Mmap(db),
                    skipped: 0,
                    source: path.display().to_string(),
                    elapsed: timer.elapsed(),
                });
            }
            Ok(LoadReport {
                db: Db::Mmap(db),
                skipped: 0,
                source: path.display().to_string(),
                elapsed: timer.elapsed(),
            })
        }
        Lookup::Sorted | Lookup::Hash => {
            let hashes = read_all_records(path)?;
            let db = Db::from_hashes(hashes, lookup);
            Ok(LoadReport {
                db,
                skipped: 0,
                source: path.display().to_string(),
                elapsed: timer.elapsed(),
            })
        }
    }
}

fn load_plh2(path: &Path) -> io::Result<DiskDb> {
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    if header[0..4] != MAGIC_V2[..] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: expected PLH2 snapshot", path.display()),
        ));
    }
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    if version != VERSION_V2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: unsupported PLH2 version {version}", path.display()),
        ));
    }
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let bloom_bytes = u64::from_le_bytes(header[24..32].try_into().unwrap()) as usize;
    let bloom_k = u32::from_le_bytes(header[32..36].try_into().unwrap());
    let bits_per_key = u32::from_le_bytes(header[36..40].try_into().unwrap());
    let n_buckets = u32::from_le_bytes(header[40..44].try_into().unwrap()) as usize;
    if n_buckets != N_BUCKETS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: unexpected bucket count {n_buckets}", path.display()),
        ));
    }
    let mut bloom_buf = vec![0u8; bloom_bytes];
    file.read_exact(&mut bloom_buf)?;
    let bloom = Bloom::from_bytes(&bloom_buf, bloom_k, bits_per_key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut index_buf = vec![0u8; BUCKET_INDEX_BYTES];
    file.read_exact(&mut index_buf)?;
    let mut buckets = Vec::with_capacity(N_BUCKETS);
    for chunk in index_buf.chunks_exact(8) {
        let off = u32::from_le_bytes(chunk[0..4].try_into().unwrap());
        let n = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
        buckets.push((off, n));
    }
    let rec_off = HEADER_LEN as u64 + bloom_bytes as u64 + BUCKET_INDEX_BYTES as u64;
    let meta = file.metadata()?;
    let expected = rec_off + count * 20;
    if meta.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: size {} != expected {expected}",
                path.display(),
                meta.len()
            ),
        ));
    }
    disable_readahead(&file);
    Ok(DiskDb {
        bloom,
        buckets,
        file,
        rec_off,
        count: count as usize,
    })
}

pub fn inspect_snapshot(path: &Path) -> io::Result<String> {
    let meta = fs::metadata(path)?;
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    let magic = std::str::from_utf8(&header[0..4]).unwrap_or("????");
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let created = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(created))
        .unwrap_or(0);
    let mut extra = String::new();
    if header[0..4] == MAGIC_V2[..] {
        let bloom_bytes = u64::from_le_bytes(header[24..32].try_into().unwrap());
        let bits_per_key = u32::from_le_bytes(header[36..40].try_into().unwrap());
        extra = format!(
            "\nbloom_mb={:.1}\nbits_per_key={bits_per_key}\nram_hint_mb~{:.0}",
            bloom_bytes as f64 / (1024.0 * 1024.0),
            bloom_bytes as f64 / (1024.0 * 1024.0) + 1.0
        );
    }
    Ok(format!(
        "snapshot={}\nsize_bytes={}\ncount={count}\ncreated_unix={created}\nage_hours={:.1}\nmagic={magic}\nversion={version}{extra}",
        path.display(),
        meta.len(),
        age as f64 / 3600.0,
    ))
}

pub fn snapshot_age_secs(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    let created = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(created);
    Ok(now.saturating_sub(created))
}

pub fn write_snapshot(path: &Path, db: &Db, bits_per_key: u32) -> io::Result<()> {
    match db {
        Db::Mmap(_) => Ok(()),
        Db::Sorted(rows) => write_plh2_from_slice(path, rows, bits_per_key),
        Db::Hash(set) => {
            let mut hashes: Vec<[u8; 20]> = set.iter().copied().collect();
            hashes.sort_unstable();
            write_plh2_from_slice(path, &hashes, bits_per_key)
        }
    }
}

fn write_plh2_from_slice(path: &Path, hashes: &[[u8; 20]], bits_per_key: u32) -> io::Result<()> {
    let tmp = path.with_extension("h160.tmp");
    write_plh2(&tmp, hashes, bits_per_key)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn write_plh2(path: &Path, hashes: &[[u8; 20]], bits_per_key: u32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut bloom = Bloom::new(hashes.len(), bits_per_key);
    let mut counts = vec![0u32; N_BUCKETS];
    for h in hashes {
        bloom.insert(h);
        counts[bucket_of(h)] += 1;
    }
    let index = index_from_counts(&counts);
    let mut file = File::create(path)?;
    write_plh2_header(&mut file, hashes.len() as u64, &bloom)?;
    write_bucket_index(&mut file, &index)?;
    for h in hashes {
        file.write_all(h)?;
    }
    file.sync_all()?;
    Ok(())
}

fn write_plh2_header(file: &mut File, count: u64, bloom: &Bloom) -> io::Result<()> {
    let mut bloom_bytes = Vec::new();
    bloom.write_to(&mut bloom_bytes);
    let mut header = [0u8; HEADER_LEN];
    header[0..4].copy_from_slice(MAGIC_V2);
    header[4..6].copy_from_slice(&VERSION_V2.to_le_bytes());
    header[8..16].copy_from_slice(&count.to_le_bytes());
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    header[16..24].copy_from_slice(&created.to_le_bytes());
    header[24..32].copy_from_slice(&(bloom_bytes.len() as u64).to_le_bytes());
    header[32..36].copy_from_slice(&bloom.k().to_le_bytes());
    header[36..40].copy_from_slice(&bloom.bits_per_key().to_le_bytes());
    header[40..44].copy_from_slice(&(N_BUCKETS as u32).to_le_bytes());
    file.write_all(&header)?;
    file.write_all(&bloom_bytes)?;
    Ok(())
}

fn write_bucket_index(file: &mut File, index: &[(u32, u32)]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(BUCKET_INDEX_BYTES);
    for (off, n) in index {
        buf.extend_from_slice(&off.to_le_bytes());
        buf.extend_from_slice(&n.to_le_bytes());
    }
    file.write_all(&buf)
}

fn index_from_counts(counts: &[u32]) -> Vec<(u32, u32)> {
    let mut index = Vec::with_capacity(N_BUCKETS);
    let mut off = 0u32;
    for &n in counts {
        index.push((off, n));
        off = off.saturating_add(n);
    }
    index
}

fn bucket_of(hash: &[u8; 20]) -> usize {
    u16::from_be_bytes([hash[0], hash[1]]) as usize
}

fn rewrite_snapshot_as_plh2(path: &Path, bits_per_key: u32) -> io::Result<()> {
    let file_len = fs::metadata(path)?.len();
    let mut src = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut header = [0u8; HEADER_LEN];
    src.read_exact(&mut header)?;
    let (count, rec_off) = record_region(&header, path, file_len)?;
    src.seek(SeekFrom::Start(rec_off))?;
    let mut bloom = Bloom::new(count as usize, bits_per_key);
    let mut counts = vec![0u32; N_BUCKETS];
    let mut rec = [0u8; 20];
    for _ in 0..count {
        src.read_exact(&mut rec)?;
        bloom.insert(&rec);
        counts[bucket_of(&rec)] += 1;
    }
    let index = index_from_counts(&counts);
    let tmp = path.with_extension("h160.tmp");
    let mut out = File::create(&tmp)?;
    write_plh2_header(&mut out, count, &bloom)?;
    write_bucket_index(&mut out, &index)?;
    src.seek(SeekFrom::Start(rec_off))?;
    io::copy(&mut src, &mut out)?;
    out.sync_all()?;
    drop(out);
    drop(src);
    fs::rename(tmp, path)?;
    Ok(())
}

fn read_all_records(path: &Path) -> io::Result<Vec<[u8; 20]>> {
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    let (count, rec_off) = record_region(&header, path, file.metadata()?.len())?;
    file.seek(SeekFrom::Start(rec_off))?;
    let mut hashes = Vec::with_capacity(count as usize);
    let mut rec = [0u8; 20];
    for _ in 0..count {
        file.read_exact(&mut rec)?;
        hashes.push(rec);
    }
    Ok(hashes)
}

fn record_region(header: &[u8; HEADER_LEN], path: &Path, file_len: u64) -> io::Result<(u64, u64)> {
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    if header[0..4] == MAGIC_V1[..] {
        return Ok((count, HEADER_LEN as u64));
    }
    if header[0..4] == MAGIC_V2[..] {
        let bloom_bytes = u64::from_le_bytes(header[24..32].try_into().unwrap());
        let rec_off = HEADER_LEN as u64 + bloom_bytes + BUCKET_INDEX_BYTES as u64;
        if file_len < rec_off + count * 20 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: truncated PLH2 records", path.display()),
            ));
        }
        return Ok((count, rec_off));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: not a PLH snapshot", path.display()),
    ))
}

fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(not(unix))]
    {
        let mut cloned = file.try_clone()?;
        cloned.seek(SeekFrom::Start(offset))?;
        cloned.read_exact(buf)
    }
}

fn advise_dontneed(file: &File, offset: u64, len: usize) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe {
            libc::posix_fadvise(
                file.as_raw_fd(),
                offset as libc::off_t,
                len as libc::off_t,
                libc::POSIX_FADV_DONTNEED,
            )
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (file, offset, len);
    }
}

fn disable_readahead(file: &File) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_RANDOM) };
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = file;
    }
}

pub fn load_pickles(cfg: &Config) -> io::Result<LoadReport> {
    let timer = Instant::now();
    let dir = &cfg.pickle_dir;
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "pickle").unwrap_or(false))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no .pickle files in {}", dir.display()),
        ));
    }

    if cfg.lookup == Lookup::Mmap {
        let partial = cfg.data_dir.join("partial");
        fs::create_dir_all(&partial)?;
        let (chunks, invalid) = pickle_to_chunks(&paths, &partial)?;
        let (sorted, dups) = merge_chunks(&chunks, &partial.join("sorted.raw"))?;
        let count = fs::metadata(&sorted)?.len() / 20;
        write_plh2_from_sorted_file(&cfg.snapshot, &sorted, count, cfg.bits_per_key)?;
        let _ = fs::remove_file(&sorted);
        for chunk in chunks {
            let _ = fs::remove_file(chunk);
        }
        let db = load_plh2(&cfg.snapshot)?;
        return Ok(LoadReport {
            db: Db::Mmap(db),
            skipped: invalid + dups,
            source: cfg.snapshot.display().to_string(),
            elapsed: timer.elapsed(),
        });
    }

    let num_threads = num_cpus::get().min(paths.len()).max(1);
    let mut shards: Vec<Vec<PathBuf>> = (0..num_threads).map(|_| Vec::new()).collect();
    for (i, p) in paths.into_iter().enumerate() {
        shards[i % num_threads].push(p);
    }
    let shard_results: Vec<(Vec<[u8; 20]>, u64)> = thread::scope(|s| {
        let handles: Vec<_> = shards
            .into_iter()
            .map(|shard| s.spawn(move || load_pickle_shard(shard)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("pickle shard panicked"))
            .collect()
    });
    let mut skipped = 0u64;
    let mut hashes = Vec::new();
    for (shard_hashes, shard_skipped) in shard_results {
        skipped += shard_skipped;
        hashes.extend(shard_hashes);
    }
    Ok(LoadReport {
        db: Db::from_hashes(hashes, cfg.lookup),
        skipped,
        source: dir.display().to_string(),
        elapsed: timer.elapsed(),
    })
}

fn pickle_to_chunks(paths: &[PathBuf], partial: &Path) -> io::Result<(Vec<PathBuf>, u64)> {
    let mut writer = ChunkWriter::new(partial, CHUNK_RECORDS);
    let mut skipped = 0u64;
    for path in paths {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        let addresses: Vec<String> =
            serde_pickle::from_slice(&bytes, Default::default()).expect("couldn't load pickle");
        for addr in &addresses {
            if let Some(h) = address_hash160(addr) {
                writer.push(h)?;
            } else {
                skipped += 1;
            }
        }
        println!("Loaded {:?}", path.file_name().unwrap_or_default());
    }
    Ok((writer.finish()?, skipped))
}

fn load_pickle_shard(paths: Vec<PathBuf>) -> (Vec<[u8; 20]>, u64) {
    let mut out = Vec::new();
    let mut skipped = 0u64;
    for path in paths {
        let mut bytes = Vec::new();
        if let Err(error) = File::open(&path).and_then(|mut f| f.read_to_end(&mut bytes)) {
            panic!("read {path:?}: {error}");
        }
        let addresses: Vec<String> =
            serde_pickle::from_slice(&bytes, Default::default()).expect("couldn't load pickle");
        for addr in &addresses {
            match address_hash160(addr) {
                Some(h) => out.push(h),
                None => skipped += 1,
            }
        }
        println!("Loaded {:?}", path.file_name().unwrap_or_default());
    }
    (out, skipped)
}

pub fn update_from_url(cfg: &Config, source_url: &str) -> io::Result<LoadReport> {
    refresh_snapshot(cfg, source_url)?;
    load_snapshot_with(&cfg.snapshot, cfg.lookup, cfg.bits_per_key)
}

pub fn refresh_snapshot(
    cfg: &Config,
    source_url: &str,
) -> io::Result<(usize, u64, std::time::Duration)> {
    fs::create_dir_all(&cfg.data_dir)?;
    let partial = cfg.data_dir.join("partial");
    fs::create_dir_all(&partial)?;
    let gz_path = partial.join("addresses.txt.gz");

    println!("Downloading {source_url}");
    download_file(source_url, &gz_path)?;
    println!("Importing funded P2PKH + P2WPKH hash160s (chunked, low RAM)");
    let timer = Instant::now();
    let (chunks, invalid) = gzip_to_chunks(&gz_path, &partial)?;
    let (sorted, dups) = merge_chunks(&chunks, &partial.join("sorted.raw"))?;
    let count = (fs::metadata(&sorted)?.len() / 20) as usize;
    write_plh2_from_sorted_file(&cfg.snapshot, &sorted, count as u64, cfg.bits_per_key)?;
    let _ = fs::remove_file(&sorted);
    let _ = fs::remove_file(&gz_path);
    for chunk in chunks {
        let _ = fs::remove_file(chunk);
    }
    Ok((count, invalid + dups, timer.elapsed()))
}

fn gzip_to_chunks(path: &Path, partial: &Path) -> io::Result<(Vec<PathBuf>, u64)> {
    let file = File::open(path)?;
    let decoder = GzDecoder::new(file);
    let reader = BufReader::with_capacity(1 << 20, decoder);
    let mut writer = ChunkWriter::new(partial, CHUNK_RECORDS);
    let mut seen = 0u64;
    let mut skipped = 0u64;
    for line in reader.lines() {
        let line = line?;
        let addr = line.split(['\t', ' ', ',']).next().unwrap_or("").trim();
        if addr.is_empty() || addr.eq_ignore_ascii_case("address") {
            continue;
        }
        seen += 1;
        if let Some(hash) = address_hash160(addr) {
            writer.push(hash)?;
        } else {
            skipped += 1;
        }
        if seen % 5_000_000 == 0 {
            println!("  scanned {seen} rows");
        }
    }
    Ok((writer.finish()?, skipped))
}

fn write_plh2_from_sorted_file(
    path: &Path,
    sorted: &Path,
    count: u64,
    bits_per_key: u32,
) -> io::Result<()> {
    let mut bloom = Bloom::new(count as usize, bits_per_key);
    let mut counts = vec![0u32; N_BUCKETS];
    {
        let mut file = BufReader::with_capacity(1 << 20, File::open(sorted)?);
        let mut rec = [0u8; 20];
        for _ in 0..count {
            file.read_exact(&mut rec)?;
            bloom.insert(&rec);
            counts[bucket_of(&rec)] += 1;
        }
    }
    let index = index_from_counts(&counts);
    let tmp = path.with_extension("h160.tmp");
    if let Some(parent) = tmp.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut out = File::create(&tmp)?;
    write_plh2_header(&mut out, count, &bloom)?;
    write_bucket_index(&mut out, &index)?;
    let mut input = File::open(sorted)?;
    io::copy(&mut input, &mut out)?;
    out.sync_all()?;
    drop(out);
    fs::rename(tmp, path)?;
    Ok(())
}

struct ChunkWriter {
    dir: PathBuf,
    cap: usize,
    buf: Vec<[u8; 20]>,
    chunks: Vec<PathBuf>,
}

impl ChunkWriter {
    fn new(dir: &Path, cap: usize) -> Self {
        Self {
            dir: dir.to_path_buf(),
            cap,
            buf: Vec::with_capacity(cap),
            chunks: Vec::new(),
        }
    }

    fn push(&mut self, hash: [u8; 20]) -> io::Result<()> {
        self.buf.push(hash);
        if self.buf.len() >= self.cap {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.buf.sort_unstable();
        self.buf.dedup();
        let path = self.dir.join(format!("chunk-{:04}.raw", self.chunks.len()));
        let mut file = File::create(&path)?;
        for h in &self.buf {
            file.write_all(h)?;
        }
        file.sync_all()?;
        self.chunks.push(path);
        self.buf.clear();
        Ok(())
    }

    fn finish(mut self) -> io::Result<Vec<PathBuf>> {
        self.flush()?;
        Ok(self.chunks)
    }
}

fn merge_chunks(chunks: &[PathBuf], out: &Path) -> io::Result<(PathBuf, u64)> {
    if chunks.is_empty() {
        File::create(out)?;
        return Ok((out.to_path_buf(), 0));
    }
    if chunks.len() == 1 {
        fs::copy(&chunks[0], out)?;
        return Ok((out.to_path_buf(), 0));
    }
    let mut readers: Vec<BufReader<File>> = chunks
        .iter()
        .map(|p| File::open(p).map(BufReader::new))
        .collect::<io::Result<_>>()?;
    let mut heap: BinaryHeap<(Reverse<[u8; 20]>, usize)> = BinaryHeap::new();
    for (i, reader) in readers.iter_mut().enumerate() {
        if let Some(rec) = read_rec(reader)? {
            heap.push((Reverse(rec), i));
        }
    }
    let mut out_file = BufWriter::new(File::create(out)?);
    let mut skipped = 0u64;
    let mut last: Option<[u8; 20]> = None;
    while let Some((Reverse(rec), i)) = heap.pop() {
        if last != Some(rec) {
            out_file.write_all(&rec)?;
            last = Some(rec);
        } else {
            skipped += 1;
        }
        if let Some(next) = read_rec(&mut readers[i])? {
            heap.push((Reverse(next), i));
        }
    }
    out_file.flush()?;
    Ok((out.to_path_buf(), skipped))
}

fn read_rec(reader: &mut BufReader<File>) -> io::Result<Option<[u8; 20]>> {
    let mut rec = [0u8; 20];
    match reader.read_exact(&mut rec) {
        Ok(()) => Ok(Some(rec)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn download_file(url: &str, dest: &Path) -> io::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let mut response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let tmp = dest.with_extension("gz.tmp");
    let mut file = File::create(&tmp)?;
    io::copy(&mut response, &mut file)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, dest)?;
    Ok(())
}

pub fn prepare_from_pickles(cfg: &Config) -> io::Result<LoadReport> {
    load_pickles(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn address_hash160_decodes_p2pkh_and_p2wpkh() {
        let key1_hash160: [u8; 20] = [
            0x75, 0x1e, 0x76, 0xe8, 0x19, 0x91, 0x96, 0xd4, 0x54, 0x94, 0x1c, 0x45, 0xd1, 0xb3,
            0xa3, 0x23, 0xf1, 0x43, 0x3b, 0xd6,
        ];
        assert_eq!(
            address_hash160("1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH"),
            Some(key1_hash160),
            "P2PKH"
        );
        assert_eq!(
            address_hash160("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            Some(key1_hash160),
            "P2WPKH"
        );
        assert_eq!(
            address_hash160("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"),
            None,
            "P2SH"
        );
        assert_eq!(
            address_hash160("bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"),
            None,
            "P2TR"
        );
    }

    fn unique_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("plutus-snap-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plh2_mmap_matches_hashset_and_stays_compact() {
        let dir = unique_dir();
        let path = dir.join("addresses.h160");
        let mut hashes = Vec::new();
        for i in 0..8_000u32 {
            let mut h = [0u8; 20];
            h[0..4].copy_from_slice(&i.to_be_bytes());
            h[4..8].copy_from_slice(&(i.wrapping_mul(0x51ed)).to_le_bytes());
            hashes.push(h);
        }
        hashes.sort_unstable();
        hashes.dedup();
        write_plh2_from_slice(&path, &hashes, 16).unwrap();

        let loaded = load_snapshot_with(&path, Lookup::Mmap, 16).unwrap();
        assert_eq!(loaded.db.lookup_name(), "mmap");
        assert_eq!(loaded.db.len(), hashes.len());
        assert!(
            loaded.db.ram_bytes() < 2 * 1024 * 1024,
            "mmap RAM {} should stay well under 2MB for 8k keys",
            loaded.db.ram_bytes()
        );
        for h in &hashes {
            assert!(loaded.db.contains(h), "false negative on stored key");
        }
        let mut absent = 0;
        for i in 0..8_000u32 {
            let mut h = [0xffu8; 20];
            h[16..20].copy_from_slice(&i.to_le_bytes());
            if !loaded.db.contains(&h) {
                absent += 1;
            }
        }
        assert!(absent > 7_900, "too many false positives: absent={absent}");
        let info = inspect_snapshot(&path).unwrap();
        assert!(info.contains("magic=PLH2"));
        let _ = fs::remove_dir_all(dir);
    }

    fn write_plh1(path: &Path, hashes: &[[u8; 20]]) {
        let mut header = [0u8; HEADER_LEN];
        header[0..4].copy_from_slice(MAGIC_V1);
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[8..16].copy_from_slice(&(hashes.len() as u64).to_le_bytes());
        let mut file = File::create(path).unwrap();
        file.write_all(&header).unwrap();
        for h in hashes {
            file.write_all(h).unwrap();
        }
        file.sync_all().unwrap();
    }

    #[test]
    fn plh1_converts_to_plh2_without_false_negatives() {
        let dir = unique_dir();
        let path = dir.join("addresses.h160");
        let mut hashes = Vec::new();
        for i in 0..1_200u32 {
            let mut h = [0u8; 20];
            h[0..4].copy_from_slice(&i.to_be_bytes());
            hashes.push(h);
        }
        hashes.sort_unstable();
        write_plh1(&path, &hashes);
        let loaded = load_snapshot_with(&path, Lookup::Mmap, 14).unwrap();
        assert_eq!(loaded.db.lookup_name(), "mmap");
        assert_eq!(loaded.db.len(), hashes.len());
        for h in &hashes {
            assert!(loaded.db.contains(h));
        }
        let info = inspect_snapshot(&path).unwrap();
        assert!(info.contains("magic=PLH2"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mmap_binary_searches_a_dense_bucket() {
        let dir = unique_dir();
        let path = dir.join("addresses.h160");
        let mut hashes = Vec::new();
        for i in 0..400u32 {
            let mut h = [0xABu8; 20];
            h[2..6].copy_from_slice(&i.to_be_bytes());
            hashes.push(h);
        }
        hashes.sort_unstable();
        write_plh2_from_slice(&path, &hashes, 16).unwrap();
        let loaded = load_snapshot_with(&path, Lookup::Mmap, 16).unwrap();
        for h in &hashes {
            assert!(loaded.db.contains(h), "missed key in dense bucket");
        }
        let mut absent = [0xABu8; 20];
        absent[2..6].copy_from_slice(&999u32.to_be_bytes());
        assert!(!loaded.db.contains(&absent));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sorted_and_hash_still_roundtrip() {
        let dir = unique_dir();
        let path = dir.join("addresses.h160");
        let hashes = vec![[1u8; 20], [2u8; 20], [9u8; 20]];
        write_plh2_from_slice(&path, &hashes, 16).unwrap();
        let sorted = load_snapshot_with(&path, Lookup::Sorted, 16).unwrap();
        assert!(sorted.db.contains(&[1u8; 20]));
        assert!(!sorted.db.contains(&[7u8; 20]));
        let hashed = load_snapshot_with(&path, Lookup::Hash, 16).unwrap();
        assert!(hashed.db.contains(&[9u8; 20]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn external_sort_dedups_chunks() {
        let dir = unique_dir();
        let mut writer = ChunkWriter::new(&dir, 4);
        for v in [3u8, 1, 2, 1, 3, 2, 9] {
            writer.push([v; 20]).unwrap();
        }
        let chunks = writer.finish().unwrap();
        let (sorted, _skipped) = merge_chunks(&chunks, &dir.join("sorted.raw")).unwrap();
        let data = fs::read(&sorted).unwrap();
        assert_eq!(data.len() % 20, 0);
        let mut got = Vec::new();
        for chunk in data.chunks_exact(20) {
            got.push(chunk[0]);
        }
        assert_eq!(got, vec![1, 2, 3, 9]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mmap_ram_under_full_table_at_64k_keys() {
        let dir = unique_dir();
        let path = dir.join("addresses.h160");
        let mut hashes = Vec::with_capacity(64_000);
        for i in 0..64_000u32 {
            let mut h = [0u8; 20];
            h[0..4].copy_from_slice(&i.to_be_bytes());
            h[4..8].copy_from_slice(&(i.wrapping_mul(0x9e37_79b9)).to_le_bytes());
            hashes.push(h);
        }
        hashes.sort_unstable();
        write_plh2_from_slice(&path, &hashes, 16).unwrap();
        let loaded = load_snapshot_with(&path, Lookup::Mmap, 16).unwrap();
        assert!(
            loaded.db.ram_bytes() < hashes.len() * 20,
            "mmap RAM {} should be below full table {}",
            loaded.db.ram_bytes(),
            hashes.len() * 20
        );
        assert!(loaded.db.contains(&hashes[0]));
        assert!(loaded.db.contains(&hashes[63_999]));
        let rebuilt = load_snapshot_with(&path, Lookup::Mmap, 18).unwrap();
        assert!(rebuilt.db.contains(&hashes[1234]));
        let _ = fs::remove_dir_all(dir);
    }
}
