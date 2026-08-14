use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static HIT_LOCK: Mutex<()> = Mutex::new(());

/// Append a finding locally and fsync it. Never called with a network client.
pub fn persist(path: &Path, record: &str) -> io::Result<()> {
    let _guard = HIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    writeln!(file, "# ts={ts}")?;
    file.write_all(record.as_bytes())?;
    if !record.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn persist_creates_missing_file_and_parent() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("plutus-hit-{unique}"));
        let path = dir.join("hits.txt");
        persist(&path, "example-record\n").unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("example-record"));
        let _ = fs::remove_dir_all(dir);
    }
}
