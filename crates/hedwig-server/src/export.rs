use crate::{BlockStore, Database};
use anyhow::{Context, ensure};
use hedwig_core::CloudPath;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub current_files: usize,
    pub historical_files: usize,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorStatus {
    pub destination: String,
    pub updated_at: String,
    pub current_files: usize,
    pub bytes_written: u64,
}

pub async fn refresh_read_only_mirror(
    data_dir: &Path,
    destination: &Path,
) -> anyhow::Result<MirrorStatus> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let name = destination
        .file_name()
        .context("Spiegelziel braucht einen Verzeichnisnamen")?;
    let destination = parent.join(name);
    let staging = parent.join(format!(
        ".{}.mirror-{}",
        name.to_string_lossy(),
        Uuid::now_v7()
    ));
    let previous = parent.join(format!(".{}.mirror-previous", name.to_string_lossy()));
    let report = export_directory_tree(data_dir, &staging, false).await?;
    let status = MirrorStatus {
        destination: destination.display().to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        current_files: report.current_files,
        bytes_written: report.bytes_written,
    };
    tokio::fs::write(
        staging.join("mirror-status.json"),
        serde_json::to_vec_pretty(&status)?,
    )
    .await?;
    set_tree_read_only(&staging, true)?;
    if previous.exists() {
        set_tree_read_only(&previous, false)?;
        fs::remove_dir_all(&previous)?;
    }
    if destination.exists() {
        fs::rename(&destination, &previous)?;
    }
    if let Err(error) = fs::rename(&staging, &destination) {
        if previous.exists() {
            let _ = fs::rename(&previous, &destination);
        }
        return Err(error.into());
    }
    if previous.exists() {
        set_tree_read_only(&previous, false)?;
        fs::remove_dir_all(previous)?;
    }
    Ok(status)
}

pub fn read_mirror_status(destination: &Path) -> anyhow::Result<Option<MirrorStatus>> {
    match fs::read(destination.join("mirror-status.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn set_tree_read_only(root: &Path, read_only: bool) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in walkdir::WalkDir::new(root).contents_first(read_only) {
        let entry = entry?;
        let mode = if entry.file_type().is_dir() {
            if read_only { 0o555 } else { 0o755 }
        } else if read_only {
            0o444
        } else {
            0o644
        };
        fs::set_permissions(entry.path(), fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_tree_read_only(root: &Path, read_only: bool) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(root).contents_first(read_only) {
        let entry = entry?;
        let mut permissions = fs::metadata(entry.path())?.permissions();
        permissions.set_readonly(read_only);
        fs::set_permissions(entry.path(), permissions)?;
    }
    Ok(())
}

pub async fn export_directory_tree(
    data_dir: &Path,
    destination: &Path,
    include_pinned: bool,
) -> anyhow::Result<ExportReport> {
    let source = data_dir.canonicalize()?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let name = destination
        .file_name()
        .context("Export-Ziel braucht einen Verzeichnisnamen")?;
    let destination = parent.join(name);
    ensure!(
        !destination.starts_with(&source),
        "Export-Ziel darf nicht im laufenden Datenbestand liegen"
    );
    ensure!(!destination.exists(), "Export-Ziel existiert bereits");
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(source.join("maintenance.lock"))?;
    fs2::FileExt::lock_shared(&lock)?;
    let staging = parent.join(format!(
        ".{}.export-{}",
        name.to_string_lossy(),
        Uuid::now_v7()
    ));
    fs::create_dir(&staging)?;
    let result = async {
        let database = Database::open(source.join("metadata.db"))?;
        let entries = database.export_entries(include_pinned)?;
        let blocks = BlockStore::new(source.join("blocks")).await?;
        let mut report = ExportReport {
            current_files: 0,
            historical_files: 0,
            bytes_written: 0,
        };
        for entry in entries {
            let cloud_path = CloudPath::parse(entry.path.clone())?;
            let space = format!("{}--{}", safe_component(&entry.space_name), entry.space_id);
            let relative = if entry.current {
                report.current_files += 1;
                PathBuf::from(space)
                    .join("current")
                    .join(cloud_path.as_str())
            } else {
                report.historical_files += 1;
                let source_path = Path::new(cloud_path.as_str());
                let file_name = source_path
                    .file_name()
                    .context("Dateiname fehlt")?
                    .to_string_lossy();
                let version_name = format!(
                    "{} ({})--{}",
                    file_name,
                    safe_component(entry.pinned_name.as_deref().unwrap_or("Version")),
                    &entry.version_id[..8],
                );
                PathBuf::from(space)
                    .join("versions")
                    .join(source_path.parent().unwrap_or_else(|| Path::new("")))
                    .join(version_name)
            };
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let temporary = target.with_extension(format!("{}.partial", Uuid::now_v7()));
            let mut output = tokio::fs::File::create(&temporary).await?;
            for block in entry.blocks {
                let content = blocks.read(&entry.space_id, &block.hash).await?;
                ensure!(
                    content.len() == block.size as usize,
                    "Blockgröße stimmt nicht: {}",
                    block.hash
                );
                ensure!(
                    blake3::hash(&content).to_hex().as_str() == block.hash,
                    "Blockhash stimmt nicht: {}",
                    block.hash
                );
                output.write_all(&content).await?;
                report.bytes_written += content.len() as u64;
            }
            output.sync_all().await?;
            drop(output);
            tokio::fs::rename(temporary, target).await?;
        }
        tokio::fs::write(
            staging.join("export-manifest.json"),
            serde_json::to_vec_pretty(&report)?,
        )
        .await?;
        fs::rename(&staging, &destination)?;
        Ok::<_, anyhow::Error>(report)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn safe_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches([' ', '.']);
    if sanitized.is_empty() {
        "Space".into()
    } else {
        sanitized.chars().take(100).collect()
    }
}
