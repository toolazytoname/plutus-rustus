//! Watch-only funded-address snapshot monitor.
//!
//! This binary deliberately operates only on a caller-supplied watchlist and a
//! local TSV snapshot. It does not generate, import, transmit, or act on
//! private keys.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, SystemTime};

use bitcoin::Address;
use reqwest::blocking::Client;

const DEFAULT_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug)]
struct Options {
    watchlist: PathBuf,
    snapshot: PathBuf,
    interval: Duration,
    once: bool,
    include_addresses: bool,
}

#[derive(Debug)]
struct SnapshotReport {
    rows_scanned: u64,
    funded_watched: BTreeSet<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("plutus-watch: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = parse_options(env::args().skip(1))?;
    let watchlist = load_watchlist(&options.watchlist)?;
    if watchlist.is_empty() {
        return Err("watchlist contains no addresses".into());
    }

    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    let mut previous = BTreeSet::new();

    loop {
        let report = check_snapshot(&options.snapshot, &watchlist)?;
        let added: Vec<_> = report
            .funded_watched
            .difference(&previous)
            .cloned()
            .collect();
        let removed: Vec<_> = previous
            .difference(&report.funded_watched)
            .cloned()
            .collect();

        let message = format_report(&options, watchlist.len(), &report, &added, &removed)?;
        println!("{message}");
        notify_if_configured(&client, "Plutus watch status", &message)?;

        previous = report.funded_watched;
        if options.once {
            return Ok(());
        }
        thread::sleep(options.interval);
    }
}

fn parse_options(arguments: impl Iterator<Item = String>) -> Result<Options, Box<dyn Error>> {
    let mut watchlist = None;
    let mut snapshot = None;
    let mut interval = DEFAULT_INTERVAL;
    let mut once = false;
    let mut include_addresses = false;
    let mut arguments = arguments.peekable();

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--watchlist" => watchlist = Some(next_value(&mut arguments, "--watchlist")?),
            "--snapshot" => snapshot = Some(next_value(&mut arguments, "--snapshot")?),
            "--interval-seconds" => {
                let seconds = next_value(&mut arguments, "--interval-seconds")?.parse::<u64>()?;
                if seconds == 0 {
                    return Err("--interval-seconds must be greater than zero".into());
                }
                interval = Duration::from_secs(seconds);
            }
            "--once" => once = true,
            "--include-addresses" => include_addresses = true,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(Options {
        watchlist: watchlist
            .map(PathBuf::from)
            .ok_or("--watchlist is required; pass a list of authorised addresses")?,
        snapshot: snapshot
            .map(PathBuf::from)
            .ok_or("--snapshot is required; pass a local TSV snapshot")?,
        interval,
        once,
        include_addresses,
    })
}

fn next_value(
    arguments: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String, Box<dyn Error>> {
    arguments
        .next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn print_usage() {
    println!(
        "Usage: plutus-watch --watchlist WATCHLIST --snapshot SNAPSHOT [OPTIONS]\n\
         \n\
         Options:\n\
           --once                    Check once and exit\n\
           --interval-seconds N      Heartbeat interval (default: 21600)\n\
           --include-addresses       Include changed addresses in stdout/webhook\n\
           -h, --help                Show this help\n\
         \n\
         Notifications are optional. Set PLUTUS_WEBHOOK_URL for a generic POST\n\
         endpoint, or PLUTUS_SERVERCHAN_KEY for ServerChan. Tokens are never\n\
         printed by this program."
    );
}

fn load_watchlist(path: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let file = File::open(path)?;
    let mut addresses = BTreeSet::new();

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let address = line.trim();
        if address.is_empty() || address.starts_with('#') {
            continue;
        }
        Address::from_str(address).map_err(|error| {
            format!(
                "invalid address at {}:{}: {error}",
                path.display(),
                index + 1
            )
        })?;
        if !addresses.insert(address.to_owned()) {
            return Err(format!("duplicate address at {}:{}", path.display(), index + 1).into());
        }
    }

    Ok(addresses)
}

fn check_snapshot(
    path: &Path,
    watchlist: &BTreeSet<String>,
) -> Result<SnapshotReport, Box<dyn Error>> {
    let file = File::open(path)?;
    let mut rows_scanned = 0;
    let mut funded_watched = BTreeSet::new();

    for line in BufReader::new(file).lines() {
        let line = line?;
        let mut columns = line.split('\t');
        let Some(address) = columns.next() else {
            continue;
        };
        let address = address.trim();
        if address.eq_ignore_ascii_case("address") {
            continue;
        }
        rows_scanned += 1;
        if !watchlist.contains(address) {
            continue;
        }

        let balance = columns.next().unwrap_or("1");
        if balance_is_positive(balance) {
            funded_watched.insert(address.to_owned());
        }
    }

    Ok(SnapshotReport {
        rows_scanned,
        funded_watched,
    })
}

fn balance_is_positive(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    value
        .parse::<u128>()
        .map(|balance| balance > 0)
        .or_else(|_| value.parse::<f64>().map(|balance| balance > 0.0))
        .unwrap_or(false)
}

fn format_report(
    options: &Options,
    watchlist_count: usize,
    report: &SnapshotReport,
    added: &[String],
    removed: &[String],
) -> Result<String, Box<dyn Error>> {
    let modified = std::fs::metadata(&options.snapshot)?.modified()?;
    let age = SystemTime::now().duration_since(modified)?.as_secs();
    let mut message = format!(
        "snapshot={} age_seconds={} rows_scanned={} watchlist={} funded={} added={} removed={}",
        options.snapshot.display(),
        age,
        report.rows_scanned,
        watchlist_count,
        report.funded_watched.len(),
        added.len(),
        removed.len(),
    );

    if options.include_addresses && (!added.is_empty() || !removed.is_empty()) {
        if !added.is_empty() {
            message.push_str(&format!(" added_addresses={}", added.join(",")));
        }
        if !removed.is_empty() {
            message.push_str(&format!(" removed_addresses={}", removed.join(",")));
        }
    }

    Ok(message)
}

fn notify_if_configured(client: &Client, title: &str, message: &str) -> Result<(), Box<dyn Error>> {
    if let Ok(url) = env::var("PLUTUS_WEBHOOK_URL") {
        let response = client
            .post(url)
            .header("content-type", "text/plain; charset=utf-8")
            .body(message.to_owned())
            .send()?;
        if !response.status().is_success() {
            return Err(format!("webhook returned {}", response.status()).into());
        }
    }

    if let Ok(key) = env::var("PLUTUS_SERVERCHAN_KEY") {
        let url = format!("https://sctapi.ftqq.com/{key}.send");
        let response = client
            .post(url)
            .form(&[("text", title), ("desp", message)])
            .send()?;
        if !response.status().is_success() {
            return Err(format!("ServerChan returned {}", response.status()).into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_parser_handles_integer_decimal_and_zero() {
        assert!(balance_is_positive("1"));
        assert!(balance_is_positive("0.00000001"));
        assert!(!balance_is_positive("0"));
        assert!(!balance_is_positive("0.0"));
        assert!(!balance_is_positive("not-a-balance"));
    }

    #[test]
    fn options_require_explicit_paths() {
        assert!(parse_options(std::iter::empty()).is_err());
        assert!(parse_options(
            ["--watchlist", "watchlist.txt", "--snapshot", "snapshot.tsv"]
                .into_iter()
                .map(String::from),
        )
        .is_ok());
    }
}
