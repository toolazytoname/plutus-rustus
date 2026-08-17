//! Durable "please look at this hit" queue.
//!
//! Findings already fsync the private key. This file only remembers the
//! *address* so alerts can keep firing after a restart until the operator
//! acks them. Never store secrets here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

static LOCK: Mutex<()> = Mutex::new(());

const PENDING_FILE: &str = "pending-hits.json";
const ACK_FILE: &str = "hits.ack";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingHit {
    pub address: String,
    pub compressed: bool,
    pub first_unix: u64,
    pub last_unix: u64,
    pub sent: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickAction {
    Send,
    Wait,
    Done,
}

/// `max == 0` means keep going until ack. `interval_secs == 0` means one shot.
pub fn tick_action(item: &PendingHit, now: u64, interval_secs: u64, max: u32) -> TickAction {
    if max > 0 && item.sent >= max {
        return TickAction::Done;
    }
    if item.sent == 0 {
        return TickAction::Send;
    }
    if interval_secs == 0 {
        return TickAction::Done;
    }
    if now.saturating_sub(item.last_unix) >= interval_secs {
        TickAction::Send
    } else {
        TickAction::Wait
    }
}

pub fn pending_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PENDING_FILE)
}

pub fn ack_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ACK_FILE)
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn enqueue(data_dir: &Path, address: &str, compressed: bool, now: u64) -> io::Result<()> {
    let _guard = lock();
    let mut items = load_unlocked(data_dir)?;
    if items
        .iter()
        .any(|item| item.address == address && item.compressed == compressed)
    {
        return Ok(());
    }
    items.push(PendingHit {
        address: address.to_owned(),
        compressed,
        first_unix: now,
        last_unix: 0,
        sent: 0,
    });
    save_unlocked(data_dir, &items)
}

pub fn apply_ack(data_dir: &Path, now: u64) -> io::Result<usize> {
    let _guard = lock();
    apply_ack_unlocked(data_dir, now)
}

pub fn due(data_dir: &Path, now: u64, interval_secs: u64, max: u32) -> io::Result<Vec<PendingHit>> {
    let _guard = lock();
    apply_ack_unlocked(data_dir, now)?;
    let items = load_unlocked(data_dir)?;
    Ok(items
        .into_iter()
        .filter(|item| tick_action(item, now, interval_secs, max) == TickAction::Send)
        .collect())
}

pub fn mark_sent(data_dir: &Path, address: &str, compressed: bool, now: u64) -> io::Result<()> {
    let _guard = lock();
    let mut items = load_unlocked(data_dir)?;
    if let Some(item) = items
        .iter_mut()
        .find(|item| item.address == address && item.compressed == compressed)
    {
        item.sent = item.sent.saturating_add(1);
        item.last_unix = now;
    }
    save_unlocked(data_dir, &items)
}

/// Stop repeating. Safe to call while the engine is running: the reporter
/// picks up `hits.ack` on the next tick and drops anything older than `now`.
pub fn ack_now(data_dir: &Path) -> io::Result<usize> {
    let now = unix_now();
    fs::create_dir_all(data_dir)?;
    fs::write(ack_path(data_dir), format!("{now}\n"))?;
    apply_ack(data_dir, now)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn apply_ack_unlocked(data_dir: &Path, now: u64) -> io::Result<usize> {
    let path = ack_path(data_dir);
    if !path.is_file() {
        return Ok(0);
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let ack_unix = raw.trim().parse::<u64>().unwrap_or(now);
    let mut items = load_unlocked(data_dir)?;
    let before = items.len();
    items.retain(|item| item.first_unix > ack_unix);
    let dropped = before - items.len();
    save_unlocked(data_dir, &items)?;
    let _ = fs::remove_file(path);
    Ok(dropped)
}

fn load_unlocked(data_dir: &Path) -> io::Result<Vec<PendingHit>> {
    let path = pending_path(data_dir);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn save_unlocked(data_dir: &Path, items: &[PendingHit]) -> io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let path = pending_path(data_dir);
    let body = serde_json::to_vec_pretty(items)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn unique_dir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("plutus-pending-{n}"))
    }

    fn sample(sent: u32, last_unix: u64) -> PendingHit {
        PendingHit {
            address: "1BgGZ9tcN4rm9KBzDn7KprQz87SZ26SAMH".into(),
            compressed: true,
            first_unix: 100,
            last_unix,
            sent,
        }
    }

    #[test]
    fn first_send_is_immediate() {
        assert_eq!(tick_action(&sample(0, 0), 1_000, 120, 24), TickAction::Send);
    }

    #[test]
    fn waits_until_interval() {
        assert_eq!(
            tick_action(&sample(1, 1_000), 1_100, 120, 24),
            TickAction::Wait
        );
        assert_eq!(
            tick_action(&sample(1, 1_000), 1_120, 120, 24),
            TickAction::Send
        );
    }

    #[test]
    fn zero_interval_is_one_shot() {
        assert_eq!(
            tick_action(&sample(1, 1_000), 9_999, 0, 24),
            TickAction::Done
        );
    }

    #[test]
    fn zero_max_means_until_ack() {
        assert_eq!(
            tick_action(&sample(99, 1_000), 1_200, 120, 0),
            TickAction::Send
        );
    }

    #[test]
    fn hits_max_then_stops() {
        assert_eq!(
            tick_action(&sample(24, 1_000), 9_999, 120, 24),
            TickAction::Done
        );
    }

    #[test]
    fn enqueue_roundtrip_and_dedup() {
        let dir = unique_dir();
        enqueue(&dir, "addr-a", true, 10).unwrap();
        enqueue(&dir, "addr-a", true, 11).unwrap();
        enqueue(&dir, "addr-a", false, 12).unwrap();
        let due = due(&dir, 20, 120, 24).unwrap();
        assert_eq!(due.len(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ack_drops_existing_but_keeps_later_hits() {
        let dir = unique_dir();
        enqueue(&dir, "old", true, 10).unwrap();
        fs::write(ack_path(&dir), "50\n").unwrap();
        enqueue(&dir, "new", true, 80).unwrap();
        let dropped = apply_ack(&dir, 80).unwrap();
        assert_eq!(dropped, 1);
        let due = due(&dir, 90, 120, 24).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].address, "new");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mark_sent_advances_counter() {
        let dir = unique_dir();
        enqueue(&dir, "addr-a", true, 10).unwrap();
        mark_sent(&dir, "addr-a", true, 20).unwrap();
        let waiting = due(&dir, 21, 120, 24).unwrap();
        assert!(waiting.is_empty());
        let later = due(&dir, 140, 120, 24).unwrap();
        assert_eq!(later[0].sent, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_file_never_holds_a_secret() {
        let dir = unique_dir();
        enqueue(&dir, "15x5ugXCVkzTbs24mG2bu1RkpshW3FTYW8", true, 1).unwrap();
        let body = fs::read_to_string(pending_path(&dir)).unwrap();
        assert!(body.contains("15x5ugXCVkzTbs24mG2bu1RkpshW3FTYW8"));
        assert!(!body.to_ascii_lowercase().contains("wif"));
        assert!(!body.contains("private"));
        let _ = fs::remove_dir_all(dir);
    }
}
