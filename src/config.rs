use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const DEFAULT_SOURCE: &str = "http://addresses.loyce.club/Bitcoin_addresses_LATEST.txt.gz";
const DEFAULT_PICKLE: &str = "database/JUL_12_2026";
const DEFAULT_BARK_SERVER: &str = "https://api.day.app";

#[derive(Debug, Clone)]
pub struct Config {
    pub threads: usize,
    pub check_uncompressed: bool,
    pub walk_span: u64,
    pub cpu_percent: u8,
    pub lookup: Lookup,
    pub bits_per_key: u32,
    pub data_dir: PathBuf,
    pub snapshot: PathBuf,
    pub pickle_dir: PathBuf,
    pub source_url: String,
    pub auto_update: bool,
    pub max_snapshot_age_hours: u64,
    pub findings: PathBuf,
    pub status: PathBuf,
    pub heartbeat_minutes: u64,
    pub notify: NotifyConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// mmap = bloom in RAM + on-disk bucket pread (~80-120MB). Default.
    Mmap,
    /// Lower RAM than hash, still ~880MB. Kept for debugging.
    Sorted,
    /// Higher RAM (~1.3GB), slightly faster contains().
    Hash,
}

#[derive(Debug, Clone)]
pub struct NotifyConfig {
    pub provider: NotifyProvider,
    pub token_env: String,
    pub webhook_url_env: String,
    pub bark_server_env: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyProvider {
    Disabled,
    Bark,
    Webhook,
    ServerChan,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            threads: 0,
            check_uncompressed: true,
            walk_span: 1 << 30,
            cpu_percent: 100,
            lookup: Lookup::Mmap,
            bits_per_key: 16,
            data_dir: PathBuf::from("data"),
            snapshot: PathBuf::from("data/addresses.h160"),
            pickle_dir: PathBuf::from(DEFAULT_PICKLE),
            source_url: DEFAULT_SOURCE.to_owned(),
            auto_update: true,
            max_snapshot_age_hours: 30,
            findings: PathBuf::from("findings/hits.txt"),
            status: PathBuf::from("data/status.json"),
            heartbeat_minutes: 1440,
            notify: NotifyConfig {
                provider: NotifyProvider::Bark,
                token_env: "PLUTUS_BARK_KEY".to_owned(),
                webhook_url_env: "PLUTUS_WEBHOOK_URL".to_owned(),
                bark_server_env: "PLUTUS_BARK_SERVER".to_owned(),
            },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    engine: FileEngine,
    #[serde(default)]
    data: FileData,
    #[serde(default)]
    run: FileRun,
    #[serde(default)]
    notify: FileNotify,
}

#[derive(Debug, Default, Deserialize)]
struct FileEngine {
    /// low | balanced | full — applied first, then explicit fields override.
    profile: Option<String>,
    threads: Option<usize>,
    check_uncompressed: Option<bool>,
    walk_span: Option<u64>,
    cpu_percent: Option<u8>,
    lookup: Option<String>,
    bits_per_key: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct FileData {
    directory: Option<String>,
    snapshot: Option<String>,
    pickle_dir: Option<String>,
    source_url: Option<String>,
    auto_update: Option<bool>,
    max_snapshot_age_hours: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct FileRun {
    findings: Option<String>,
    status: Option<String>,
    heartbeat_minutes: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct FileNotify {
    provider: Option<String>,
    token_env: Option<String>,
    webhook_url_env: Option<String>,
    bark_server_env: Option<String>,
}

pub fn load() -> Config {
    let path = env::var("PLUTUS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"));
    load_from_path(&path)
}

pub fn load_from_path(path: &Path) -> Config {
    let mut cfg = Config::default();
    if let Ok(raw) = fs::read_to_string(path) {
        match toml::from_str::<FileConfig>(&raw) {
            Ok(file) => apply_file(&mut cfg, file),
            Err(error) => {
                eprintln!("warning: ignoring {}: {error}", path.display());
            }
        }
    }
    apply_env(&mut cfg);
    cfg
}

fn apply_profile(cfg: &mut Config, profile: &str) {
    match profile.trim().to_ascii_lowercase().as_str() {
        "low" => {
            cfg.threads = 1;
            cfg.check_uncompressed = false;
            cfg.lookup = Lookup::Mmap;
            cfg.cpu_percent = 40;
            cfg.bits_per_key = 14;
        }
        "balanced" => {
            cfg.threads = 0;
            cfg.check_uncompressed = true;
            cfg.lookup = Lookup::Mmap;
            cfg.cpu_percent = 70;
            cfg.bits_per_key = 16;
        }
        "full" => {
            cfg.threads = 0;
            cfg.check_uncompressed = true;
            cfg.lookup = Lookup::Mmap;
            cfg.cpu_percent = 100;
            cfg.bits_per_key = 18;
        }
        other => eprintln!("warning: unknown engine.profile {other:?}, ignored"),
    }
}

fn apply_file(cfg: &mut Config, file: FileConfig) {
    if let Some(profile) = file.engine.profile.as_deref() {
        apply_profile(cfg, profile);
    }
    if let Some(threads) = file.engine.threads {
        cfg.threads = threads;
    }
    if let Some(value) = file.engine.check_uncompressed {
        cfg.check_uncompressed = value;
    }
    if let Some(span) = file.engine.walk_span {
        if span > 0 {
            cfg.walk_span = span;
        }
    }
    if let Some(percent) = file.engine.cpu_percent {
        cfg.cpu_percent = percent.clamp(1, 100);
    }
    if let Some(lookup) = file.engine.lookup.as_deref() {
        cfg.lookup = parse_lookup(lookup);
    }
    if let Some(bits) = file.engine.bits_per_key {
        cfg.bits_per_key = bits.clamp(8, 32);
    }
    if let Some(dir) = file.data.directory {
        cfg.data_dir = PathBuf::from(dir);
    }
    if let Some(snapshot) = file.data.snapshot {
        cfg.snapshot = PathBuf::from(snapshot);
    }
    if let Some(pickle) = file.data.pickle_dir {
        cfg.pickle_dir = PathBuf::from(pickle);
    }
    if let Some(url) = file.data.source_url {
        if !url.is_empty() {
            cfg.source_url = url;
        }
    }
    if let Some(auto_update) = file.data.auto_update {
        cfg.auto_update = auto_update;
    }
    if let Some(hours) = file.data.max_snapshot_age_hours {
        if hours > 0 {
            cfg.max_snapshot_age_hours = hours;
        }
    }
    if let Some(findings) = file.run.findings {
        cfg.findings = PathBuf::from(findings);
    }
    if let Some(status) = file.run.status {
        cfg.status = PathBuf::from(status);
    }
    if let Some(minutes) = file.run.heartbeat_minutes {
        if minutes > 0 {
            cfg.heartbeat_minutes = minutes;
        }
    }
    if let Some(provider) = file.notify.provider {
        cfg.notify.provider = parse_provider(&provider);
    }
    if let Some(token_env) = file.notify.token_env {
        cfg.notify.token_env = token_env;
    }
    if let Some(webhook_env) = file.notify.webhook_url_env {
        cfg.notify.webhook_url_env = webhook_env;
    }
    if let Some(server_env) = file.notify.bark_server_env {
        cfg.notify.bark_server_env = server_env;
    }
}

fn apply_env(cfg: &mut Config) {
    if let Ok(threads) = env::var("PLUTUS_THREADS") {
        if let Ok(n) = threads.parse::<usize>() {
            cfg.threads = n;
        }
    }
    if let Ok(value) = env::var("PLUTUS_CHECK_UNCOMPRESSED") {
        cfg.check_uncompressed = matches!(value.as_str(), "1" | "true" | "TRUE" | "yes");
    }
    if let Ok(value) = env::var("PLUTUS_CPU_PERCENT") {
        if let Ok(n) = value.parse::<u8>() {
            cfg.cpu_percent = n.clamp(1, 100);
        }
    }
}

fn parse_provider(value: &str) -> NotifyProvider {
    match value.trim().to_ascii_lowercase().as_str() {
        "bark" => NotifyProvider::Bark,
        "webhook" => NotifyProvider::Webhook,
        "serverchan" => NotifyProvider::ServerChan,
        "disabled" | "off" | "none" => NotifyProvider::Disabled,
        _ => NotifyProvider::Disabled,
    }
}

fn parse_lookup(value: &str) -> Lookup {
    match value.trim().to_ascii_lowercase().as_str() {
        "hash" | "hashset" => Lookup::Hash,
        "sorted" => Lookup::Sorted,
        _ => Lookup::Mmap,
    }
}

pub fn worker_count(cfg: &Config) -> usize {
    if cfg.threads > 0 {
        cfg.threads
    } else {
        num_cpus::get().max(1)
    }
}

pub fn node_name() -> String {
    env::var("PLUTUS_NODE_NAME")
        .or_else(|_| env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "plutus".to_owned())
}

pub fn bark_server(cfg: &NotifyConfig) -> String {
    env::var(&cfg.bark_server_env)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BARK_SERVER.to_owned())
}

impl Lookup {
    pub fn as_str(self) -> &'static str {
        match self {
            Lookup::Mmap => "mmap",
            Lookup::Sorted => "sorted",
            Lookup::Hash => "hash",
        }
    }
}

/// Steady-state RAM hint for the funded-address table (not process RSS).
pub fn ram_hint_mb(cfg: &Config) -> u64 {
    match cfg.lookup {
        Lookup::Hash => 1300,
        Lookup::Sorted => 900,
        Lookup::Mmap => {
            let n = 44_365_067u64;
            let bloom = n.saturating_mul(u64::from(cfg.bits_per_key)).div_ceil(8);
            (bloom + 512 * 1024) / (1024 * 1024)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_product_defaults() {
        let cfg = load_from_path(Path::new("definitely-missing-plutus-config.toml"));
        assert!(cfg.check_uncompressed);
        assert_eq!(cfg.walk_span, 1 << 30);
        assert_eq!(cfg.notify.provider, NotifyProvider::Bark);
        assert_eq!(cfg.heartbeat_minutes, 1440);
        assert_eq!(cfg.lookup, Lookup::Mmap);
        assert_eq!(cfg.bits_per_key, 16);
        assert!(cfg.auto_update);
    }

    #[test]
    fn low_profile_caps_cpu_and_threads() {
        let mut cfg = Config::default();
        apply_profile(&mut cfg, "low");
        assert_eq!(cfg.threads, 1);
        assert!(!cfg.check_uncompressed);
        assert_eq!(cfg.cpu_percent, 40);
        assert_eq!(cfg.lookup, Lookup::Mmap);
        assert_eq!(cfg.bits_per_key, 14);
        assert!(ram_hint_mb(&cfg) < 120);
    }

    #[test]
    fn mmap_ram_hint_is_far_below_sorted_table() {
        let cfg = Config::default();
        assert_eq!(cfg.lookup, Lookup::Mmap);
        assert!(ram_hint_mb(&cfg) < 120);
        assert!(ram_hint_mb(&cfg) > 50);
    }
}
