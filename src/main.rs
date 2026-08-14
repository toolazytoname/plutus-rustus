use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use plutus_rustus::config::{self, Config};
use plutus_rustus::db;
use plutus_rustus::engine::{self, RunOutcome};
use plutus_rustus::notify::Notifier;

#[derive(Parser)]
#[command(
    name = "plutus-rustus",
    about = "Funded-address key-space collider with a durable local snapshot."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the collider (default if no subcommand is given).
    Run,
    /// Check config, snapshot, write paths, RAM hints, and notifier wiring.
    Doctor,
    /// Send one Bark/webhook test that contains no secrets.
    NotifyTest,
    /// Snapshot import, refresh, and inspection.
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
}

#[derive(Subcommand)]
enum DataCommand {
    /// Convert bundled pickle slices into the binary snapshot.
    Prepare,
    /// Download the latest funded-address dump and atomically replace the snapshot.
    Update {
        #[arg(long)]
        source_url: Option<String>,
    },
    /// Print snapshot header fields.
    Inspect,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = config::load();
    let result = match cli.command.unwrap_or(Command::Run) {
        Command::Run => match engine::run(&cfg) {
            Ok(RunOutcome::Stopped) => Ok(()),
            Ok(RunOutcome::Reload) => return ExitCode::from(engine::RELOAD_EXIT),
            Err(error) => Err(error),
        },
        Command::Doctor => doctor(&cfg),
        Command::NotifyTest => notify_test(&cfg),
        Command::Data {
            command: DataCommand::Prepare,
        } => data_prepare(&cfg),
        Command::Data {
            command: DataCommand::Update { source_url },
        } => data_update(&cfg, source_url),
        Command::Data {
            command: DataCommand::Inspect,
        } => data_inspect(&cfg),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("plutus-rustus: {error}");
            ExitCode::FAILURE
        }
    }
}

fn doctor(cfg: &Config) -> Result<(), String> {
    let mut ok = true;
    let threads = config::worker_count(cfg);
    println!("node={}", config::node_name());
    println!("threads={threads}");
    println!("check_uncompressed={}", cfg.check_uncompressed);
    println!("cpu_percent={}", cfg.cpu_percent);
    println!("lookup={}", cfg.lookup.as_str());
    println!("bits_per_key={}", cfg.bits_per_key);
    println!("simd={}", engine::simd_name());
    println!("auto_update={}", cfg.auto_update);
    println!("max_snapshot_age_hours={}", cfg.max_snapshot_age_hours);
    println!("heartbeat_minutes={}", cfg.heartbeat_minutes);
    println!("snapshot={}", cfg.snapshot.display());
    println!("pickle_dir={}", cfg.pickle_dir.display());
    println!("findings={}", cfg.findings.display());
    println!("status={}", cfg.status.display());

    let ram_hint_mb = config::ram_hint_mb(cfg);
    println!(
        "ram_hint_mb~{ram_hint_mb} (mmap keeps bloom+index in RAM; the 20-byte table stays on disk)"
    );
    if threads > 1 && cfg.cpu_percent >= 90 {
        println!("hint=weak VPS: set engine.profile=\"low\" or PLUTUS_CPU_PERCENT=40");
    }

    match writable_parent(&cfg.data_dir) {
        Ok(()) => println!("data_dir=writable"),
        Err(error) => {
            println!("data_dir=ERROR {error}");
            ok = false;
        }
    }
    if let Some(parent) = cfg.findings.parent() {
        match writable_parent(parent) {
            Ok(()) => println!("findings_dir=writable"),
            Err(error) => {
                println!("findings_dir=ERROR {error}");
                ok = false;
            }
        }
    }

    if cfg.snapshot.is_file() {
        match db::inspect_snapshot(&cfg.snapshot) {
            Ok(info) => println!("{info}"),
            Err(error) => {
                println!("snapshot=ERROR {error}");
                ok = false;
            }
        }
    } else if cfg.pickle_dir.is_dir() {
        println!("snapshot=missing (pickle fallback present; first run will migrate)");
    } else {
        println!("snapshot=missing (run `plutus-rustus data update` or `data prepare`)");
        ok = false;
    }

    let notifier = Notifier::from_config(&cfg.notify);
    println!("notify={}", notifier.configured_without_secret());
    if ok {
        println!("doctor=ok");
        Ok(())
    } else {
        Err("doctor found problems".into())
    }
}

fn notify_test(cfg: &Config) -> Result<(), String> {
    let notifier = Notifier::from_config(&cfg.notify);
    notifier.send_result(
        "Plutus 连通测试",
        &format!(
            "node={} provider={}",
            config::node_name(),
            notifier.configured_without_secret()
        ),
    )?;
    println!(
        "notify-test sent via {}",
        notifier.configured_without_secret()
    );
    Ok(())
}

fn writable_parent(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(path)?;
    let probe = path.join(".plutus-write-probe");
    {
        let mut file = fs::File::create(&probe)?;
        file.write_all(b"ok")?;
    }
    fs::remove_file(probe)?;
    Ok(())
}

fn data_prepare(cfg: &Config) -> Result<(), String> {
    let report = db::prepare_from_pickles(cfg).map_err(|e| e.to_string())?;
    println!(
        "Prepared {} unique hash160s in {:.2?} -> {} ({} skipped)",
        report.db.len(),
        report.elapsed,
        cfg.snapshot.display(),
        report.skipped
    );
    Ok(())
}

fn data_update(cfg: &Config, source_url: Option<String>) -> Result<(), String> {
    let url = source_url.unwrap_or_else(|| cfg.source_url.clone());
    let report = db::update_from_url(cfg, &url).map_err(|e| e.to_string())?;
    println!(
        "Updated snapshot {} with {} unique hash160s in {:.2?} ({} skipped). Restart the engine to load it.",
        cfg.snapshot.display(),
        report.db.len(),
        report.elapsed,
        report.skipped
    );
    Ok(())
}

fn data_inspect(cfg: &Config) -> Result<(), String> {
    let info = db::inspect_snapshot(&cfg.snapshot).map_err(|e| e.to_string())?;
    println!("{info}");
    Ok(())
}
