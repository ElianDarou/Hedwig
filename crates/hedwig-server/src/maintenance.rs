use crate::{BlockStore, Database};
use anyhow::Context;
use chrono::Utc;
use serde::Serialize;
use std::{fs, path::Path};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceReport {
    pub deleted_versions: usize,
    pub purged_files: usize,
    pub removed_blocks: usize,
}

pub async fn run_maintenance(data_dir: &Path) -> anyhow::Result<MaintenanceReport> {
    fs::create_dir_all(data_dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(data_dir.join("maintenance.lock"))?;
    fs2::FileExt::lock_exclusive(&lock).context("Backup oder eine andere Wartung läuft bereits")?;
    let database = Database::open(data_dir.join("metadata.db"))?;
    let now = Utc::now();
    let retention = database.apply_default_retention(now)?;
    let protected = database.protected_blocks(now)?;
    let store = BlockStore::new(data_dir.join("blocks")).await?;
    let mut removed_blocks = 0;
    for (space_id, hash) in retention.unused_blocks {
        if store.remove(&space_id, &hash).await? {
            removed_blocks += 1;
        }
    }
    for entry in WalkDir::new(store.root()).min_depth(3).max_depth(3) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(store.root())?;
        let components: Vec<_> = relative.components().collect();
        if components.len() != 3 {
            continue;
        }
        let space_id = components[0].as_os_str().to_string_lossy().to_string();
        let hash = components[2].as_os_str().to_string_lossy().to_string();
        if hash.len() != 64 || protected.contains(&(space_id.clone(), hash.clone())) {
            continue;
        }
        if store.remove(&space_id, &hash).await? {
            removed_blocks += 1;
        }
    }
    Ok(MaintenanceReport {
        deleted_versions: retention.deleted_versions,
        purged_files: retention.purged_files,
        removed_blocks,
    })
}
