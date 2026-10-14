use anyhow::Context;
use clap::Parser;
use hedwig_server::{
    AppConfig, Database, build_app, deliver_pending_webhooks, refresh_read_only_mirror,
    run_maintenance, run_scheduled_backup,
};
use std::{net::SocketAddr, path::PathBuf};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about = "Hedwig server")]
struct Arguments {
    #[arg(long, env = "HEDWIG_DATA_DIR", default_value = "./data")]
    data_dir: PathBuf,
    #[arg(long, env = "HEDWIG_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, env = "HEDWIG_WEB_DIR")]
    web_dir: Option<PathBuf>,
    #[arg(long, env = "HEDWIG_MAINTENANCE_INTERVAL_HOURS", default_value_t = 6)]
    maintenance_interval_hours: u64,
    #[arg(long, env = "HEDWIG_BACKUP_DIR")]
    backup_dir: Option<PathBuf>,
    #[arg(long, env = "HEDWIG_BACKUP_INTERVAL_HOURS", default_value_t = 24)]
    backup_interval_hours: u64,
    #[arg(long, env = "HEDWIG_BACKUP_KEEP", default_value_t = 14)]
    backup_keep: usize,
    #[arg(long, env = "HEDWIG_MIRROR_DIR")]
    mirror_dir: Option<PathBuf>,
    #[arg(long, env = "HEDWIG_MIRROR_INTERVAL_MINUTES", default_value_t = 15)]
    mirror_interval_minutes: u64,
}

fn promote_legacy_environment() {
    for (current, legacy) in [
        ("HEDWIG_DATA_DIR", "CLOUD_DATA_DIR"),
        ("HEDWIG_LISTEN", "CLOUD_LISTEN"),
        ("HEDWIG_WEB_DIR", "CLOUD_WEB_DIR"),
        (
            "HEDWIG_MAINTENANCE_INTERVAL_HOURS",
            "CLOUD_MAINTENANCE_INTERVAL_HOURS",
        ),
        ("HEDWIG_BACKUP_DIR", "CLOUD_BACKUP_DIR"),
        (
            "HEDWIG_BACKUP_INTERVAL_HOURS",
            "CLOUD_BACKUP_INTERVAL_HOURS",
        ),
        ("HEDWIG_BACKUP_KEEP", "CLOUD_BACKUP_KEEP"),
        ("HEDWIG_MIRROR_DIR", "CLOUD_MIRROR_DIR"),
        (
            "HEDWIG_MIRROR_INTERVAL_MINUTES",
            "CLOUD_MIRROR_INTERVAL_MINUTES",
        ),
    ] {
        if std::env::var_os(current).is_none()
            && let Some(value) = std::env::var_os(legacy)
        {
            // SAFETY: this runs before tracing and the async runtime start worker threads.
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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "hedwig_server=info".into()),
        )
        .init();

    let arguments = Arguments::parse();
    let data_dir = arguments.data_dir.clone();
    let mut config = AppConfig::new(arguments.data_dir);
    config.web_dir = arguments.web_dir;
    config.backup_dir = arguments.backup_dir.clone();
    config.backup_interval_hours = arguments.backup_interval_hours;
    config.backup_keep = arguments.backup_keep;
    config.mirror_dir = arguments.mirror_dir.clone();
    config.mirror_interval_minutes = arguments.mirror_interval_minutes;
    let app = build_app(config).await?;
    let webhook_database = Database::open(data_dir.join("metadata.db"))?;
    let listener = TcpListener::bind(arguments.listen)
        .await
        .with_context(|| format!("cannot listen on {}", arguments.listen))?;
    if arguments.maintenance_interval_hours > 0 {
        let maintenance_data_dir = data_dir.clone();
        let interval = std::time::Duration::from_secs(
            arguments.maintenance_interval_hours.saturating_mul(60 * 60),
        );
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match run_maintenance(&maintenance_data_dir).await {
                    Ok(report) => tracing::info!(
                        deleted_versions = report.deleted_versions,
                        purged_files = report.purged_files,
                        removed_blocks = report.removed_blocks,
                        "maintenance completed"
                    ),
                    Err(error) => tracing::error!(error = %error, "maintenance failed"),
                }
            }
        });
    }
    if let Some(backup_dir) = arguments.backup_dir {
        if arguments.backup_interval_hours == 0 {
            tracing::warn!(
                "HEDWIG_BACKUP_DIR ist gesetzt, der interne Backup-Zeitplan aber deaktiviert"
            );
        } else {
            let backup_data_dir = data_dir.clone();
            let interval = std::time::Duration::from_secs(
                arguments.backup_interval_hours.saturating_mul(60 * 60),
            );
            let keep = arguments.backup_keep.max(1);
            tokio::spawn(async move {
                loop {
                    let source = backup_data_dir.clone();
                    let target = backup_dir.clone();
                    match tokio::task::spawn_blocking(move || {
                        run_scheduled_backup(&source, &target, keep)
                    })
                    .await
                    {
                        Ok(Ok(manifest)) => tracing::info!(
                            entries = manifest.entries.len(),
                            destination = %backup_dir.display(),
                            "scheduled backup completed and verified"
                        ),
                        Ok(Err(error)) => {
                            tracing::error!(error = %error, "scheduled backup failed")
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "scheduled backup task failed")
                        }
                    }
                    tokio::time::sleep(interval).await;
                }
            });
        }
    }
    if let Some(mirror_dir) = arguments.mirror_dir {
        if arguments.mirror_interval_minutes == 0 {
            tracing::warn!(
                "HEDWIG_MIRROR_DIR ist gesetzt, der laufende Spiegel-Zeitplan aber deaktiviert"
            );
        } else {
            let mirror_data_dir = data_dir.clone();
            let interval = std::time::Duration::from_secs(
                arguments.mirror_interval_minutes.saturating_mul(60),
            );
            tokio::spawn(async move {
                loop {
                    match refresh_read_only_mirror(&mirror_data_dir, &mirror_dir).await {
                        Ok(status) => tracing::info!(
                            files = status.current_files,
                            bytes = status.bytes_written,
                            destination = %mirror_dir.display(),
                            "read-only mirror refreshed"
                        ),
                        Err(error) => tracing::error!(error = %error, "mirror refresh failed"),
                    }
                    tokio::time::sleep(interval).await;
                }
            });
        }
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if let Err(error) = deliver_pending_webhooks(&webhook_database).await {
                tracing::error!(error = %error, "webhook delivery failed");
            }
        }
    });
    tracing::info!(address = %arguments.listen, "Hedwig server started");
    axum::serve(listener, app).await.context("server stopped")
}
