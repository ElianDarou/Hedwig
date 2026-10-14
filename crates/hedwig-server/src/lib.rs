mod api;
mod backup;
mod database;
mod export;
mod maintenance;
mod storage;
mod webhook;

pub use api::build_router;
pub use backup::{
    BackupManifest, create_backup, restore_backup, run_scheduled_backup, verify_backup,
};
pub use database::Database;
pub use export::{
    ExportReport, MirrorStatus, export_directory_tree, read_mirror_status, refresh_read_only_mirror,
};
pub use maintenance::{MaintenanceReport, run_maintenance};
pub use storage::BlockStore;
pub use webhook::deliver_pending_webhooks;

use anyhow::Context;
use axum::Router;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub web_dir: Option<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub backup_interval_hours: u64,
    pub backup_keep: usize,
    pub mirror_dir: Option<PathBuf>,
    pub mirror_interval_minutes: u64,
}

impl AppConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            web_dir: None,
            backup_dir: None,
            backup_interval_hours: 0,
            backup_keep: 0,
            mirror_dir: None,
            mirror_interval_minutes: 15,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupConfiguration {
    pub destination: String,
    pub interval_hours: u64,
    pub keep: usize,
    pub same_filesystem: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorConfiguration {
    pub destination: String,
    pub interval_minutes: u64,
    pub same_filesystem: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub database: Database,
    pub blocks: BlockStore,
    pub backup: Option<BackupConfiguration>,
    pub data_dir: PathBuf,
    pub backup_gate: Arc<tokio::sync::Mutex<()>>,
    pub mirror: Option<MirrorConfiguration>,
    pub mirror_gate: Arc<tokio::sync::Mutex<()>>,
    pub change_notifier: tokio::sync::broadcast::Sender<()>,
}

pub async fn build_app(config: AppConfig) -> anyhow::Result<Router> {
    tokio::fs::create_dir_all(&config.data_dir)
        .await
        .with_context(|| format!("cannot create {}", config.data_dir.display()))?;
    let database = Database::open(config.data_dir.join("metadata.db"))?;
    let blocks = BlockStore::new(config.data_dir.join("blocks")).await?;
    let backup = if let Some(destination) = config.backup_dir {
        tokio::fs::create_dir_all(&destination).await?;
        let data = tokio::fs::canonicalize(&config.data_dir).await?;
        let destination = tokio::fs::canonicalize(destination).await?;
        anyhow::ensure!(
            !destination.starts_with(&data),
            "Backup-Ziel darf nicht im laufenden Datenbestand liegen"
        );
        Some(BackupConfiguration {
            destination: destination.display().to_string(),
            interval_hours: config.backup_interval_hours,
            keep: config.backup_keep.max(1),
            same_filesystem: same_filesystem(&data, &destination)?,
        })
    } else {
        None
    };
    let mirror = if let Some(destination) = config.mirror_dir {
        tokio::fs::create_dir_all(&destination).await?;
        let data = tokio::fs::canonicalize(&config.data_dir).await?;
        let destination = tokio::fs::canonicalize(destination).await?;
        anyhow::ensure!(
            !destination.starts_with(&data),
            "Spiegelziel darf nicht im laufenden Datenbestand liegen"
        );
        Some(MirrorConfiguration {
            destination: destination.display().to_string(),
            interval_minutes: config.mirror_interval_minutes,
            same_filesystem: same_filesystem(&data, &destination)?,
        })
    } else {
        None
    };
    let (change_notifier, _) = tokio::sync::broadcast::channel(64);
    Ok(build_router(
        Arc::new(AppState {
            database,
            blocks,
            backup,
            data_dir: config.data_dir,
            backup_gate: Arc::new(tokio::sync::Mutex::new(())),
            mirror,
            mirror_gate: Arc::new(tokio::sync::Mutex::new(())),
            change_notifier,
        }),
        config.web_dir,
    ))
}

#[cfg(unix)]
fn same_filesystem(left: &std::path::Path, right: &std::path::Path) -> anyhow::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(left)?.dev() == std::fs::metadata(right)?.dev())
}

#[cfg(not(unix))]
fn same_filesystem(_left: &std::path::Path, _right: &std::path::Path) -> anyhow::Result<bool> {
    Ok(false)
}
