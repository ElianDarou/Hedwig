use clap::{Parser, ValueEnum};
use hedwig_agent::sync::{SyncConfig, SyncMode, synchronize};
use hedwig_client::CloudClient;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Hedwig desktop synchronization agent")]
struct Arguments {
    #[arg(long, env = "HEDWIG_SERVER")]
    server: String,
    #[arg(long, env = "HEDWIG_TOKEN", hide_env_values = true)]
    token: String,
    #[arg(long, env = "HEDWIG_SPACE")]
    space: String,
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long, value_enum, default_value = "two-way")]
    mode: ModeArgument,
    #[arg(long, default_value_t = 25)]
    delete_limit: usize,
    #[arg(long, default_value_t = 20)]
    delete_percent: usize,
    /// Confirms the suspicious deletion batch for this run only.
    #[arg(long)]
    allow_deletions_once: bool,
    /// Number of files transferred concurrently. Higher values favor sync latency over load.
    #[arg(long, env = "HEDWIG_PARALLEL_TRANSFERS", default_value_t = 16)]
    parallel_transfers: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArgument {
    TwoWay,
    UploadOnly,
    DownloadOnly,
}

fn promote_legacy_environment() {
    for (current, legacy) in [
        ("HEDWIG_SERVER", "CLOUD_SERVER"),
        ("HEDWIG_TOKEN", "CLOUD_TOKEN"),
        ("HEDWIG_SPACE", "CLOUD_SPACE"),
        ("HEDWIG_PARALLEL_TRANSFERS", "CLOUD_PARALLEL_TRANSFERS"),
    ] {
        if std::env::var_os(current).is_none()
            && let Some(value) = std::env::var_os(legacy)
        {
            // SAFETY: this runs before the async runtime starts or any worker thread is created.
            unsafe { std::env::set_var(current, value) };
        }
    }
}

fn main() -> anyhow::Result<()> {
    promote_legacy_environment();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let client = CloudClient::new(&arguments.server, arguments.token)?;
    let report = synchronize(
        client,
        SyncConfig {
            space_id: arguments.space,
            root: arguments.root,
            state_dir: arguments.state_dir,
            mode: match arguments.mode {
                ModeArgument::TwoWay => SyncMode::TwoWay,
                ModeArgument::UploadOnly => SyncMode::UploadOnly,
                ModeArgument::DownloadOnly => SyncMode::DownloadOnly,
            },
            delete_limit: arguments.delete_limit,
            delete_percent: arguments.delete_percent,
            allow_deletions_once: arguments.allow_deletions_once,
            selective_excluded: Vec::new(),
            parallel_transfers: arguments.parallel_transfers,
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if report.deletion_pause {
        std::process::exit(2);
    }
    Ok(())
}
