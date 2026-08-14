use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Status {
    pub started_unix: u64,
    pub now_unix: u64,
    pub uptime_secs: u64,
    pub keys_checked: u64,
    pub keys_per_sec_avg: f64,
    pub keys_per_sec_inst: f64,
    pub hits: u64,
    pub db_size: usize,
    pub threads: usize,
    pub check_uncompressed: bool,
    pub simd: &'static str,
    pub snapshot: String,
    pub lookup: String,
    pub ram_bytes: usize,
    pub cpu_percent: u8,
    pub node: String,
    pub running: bool,
    pub reload_requested: bool,
}

pub fn write_atomic(path: &Path, status: &Status) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let body = serde_json::to_vec_pretty(status)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
