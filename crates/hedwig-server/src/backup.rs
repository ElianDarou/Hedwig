use crate::Database;
use anyhow::{Context, bail, ensure};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const MANIFEST_NAME: &str = "backup-manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: u32,
    pub created_at: String,
    pub entries: Vec<BackupEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub path: String,
    pub size: u64,
    pub blake3: String,
}

pub fn create_backup(data_dir: &Path, destination: &Path) -> anyhow::Result<BackupManifest> {
    let source = data_dir
        .canonicalize()
        .with_context(|| format!("Datenverzeichnis {} fehlt", data_dir.display()))?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let name = destination
        .file_name()
        .context("Backup-Ziel braucht einen Verzeichnisnamen")?;
    let destination = parent.join(name);
    ensure!(
        !destination.starts_with(&source),
        "Backup-Ziel darf nicht im laufenden Datenbestand liegen"
    );
    ensure!(!destination.exists(), "Backup-Ziel existiert bereits");
    let maintenance_lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(source.join("maintenance.lock"))?;
    fs2::FileExt::lock_shared(&maintenance_lock)?;

    let staging = parent.join(format!(
        ".{}.partial-{}",
        name.to_string_lossy(),
        Uuid::now_v7()
    ));
    fs::create_dir(&staging)?;
    let result = (|| {
        let database = Database::open(source.join("metadata.db"))?;
        database.snapshot_to(&staging.join("metadata.db"))?;
        // Laufstatus ist betriebliche Telemetrie des Quellservers und darf nach einem Restore
        // nicht fälschlich als noch laufende Sicherung erscheinen.
        Connection::open(staging.join("metadata.db"))?.execute("DELETE FROM backup_runs", [])?;
        copy_tree(&source.join("blocks"), &staging.join("blocks"))?;
        let mut entries = collect_entries(&staging)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = BackupManifest {
            format: 1,
            created_at: Utc::now().to_rfc3339(),
            entries,
        };
        write_json_atomic(&staging.join(MANIFEST_NAME), &manifest)?;
        fs::rename(&staging, &destination)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

pub fn run_scheduled_backup(
    data_dir: &Path,
    backup_root: &Path,
    keep: usize,
) -> anyhow::Result<BackupManifest> {
    fs::create_dir_all(backup_root)?;
    let name = format!(
        "hedwig-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        Uuid::now_v7()
    );
    let destination = backup_root.join(name);
    let database = Database::open(data_dir.join("metadata.db"))?;
    let run_id = database.start_backup_run(&destination)?;
    let result = (|| {
        let manifest = create_backup(data_dir, &destination)?;
        verify_backup(&destination)?;
        prune_scheduled_backups(backup_root, keep.max(1))?;
        Ok::<_, anyhow::Error>(manifest)
    })();
    match &result {
        Ok(manifest) => {
            let bytes = manifest.entries.iter().map(|entry| entry.size).sum::<u64>();
            database.finish_backup_run(
                &run_id,
                Some(manifest.entries.len() as i64),
                Some(i64::try_from(bytes).unwrap_or(i64::MAX)),
                None,
            )?;
        }
        Err(error) => database.finish_backup_run(&run_id, None, None, Some(&error.to_string()))?,
    }
    result
}

fn prune_scheduled_backups(root: &Path, keep: usize) -> anyhow::Result<()> {
    let mut backups = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("hedwig-"))
        .filter(|entry| entry.path().join(MANIFEST_NAME).is_file())
        .collect::<Vec<_>>();
    backups.sort_by_key(|entry| entry.file_name());
    let remove = backups.len().saturating_sub(keep);
    for entry in backups.into_iter().take(remove) {
        fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

pub fn verify_backup(source: &Path) -> anyhow::Result<BackupManifest> {
    let manifest: BackupManifest = serde_json::from_slice(
        &fs::read(source.join(MANIFEST_NAME)).context("Backup-Manifest fehlt")?,
    )?;
    ensure!(manifest.format == 1, "Unbekanntes Backup-Format");
    for entry in &manifest.entries {
        let relative = safe_relative(&entry.path)?;
        let path = source.join(relative);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("Backup-Datei {} fehlt", entry.path))?;
        ensure!(
            metadata.file_type().is_file(),
            "Ungültiger Dateityp: {}",
            entry.path
        );
        ensure!(
            metadata.len() == entry.size,
            "Falsche Größe: {}",
            entry.path
        );
        ensure!(
            hash_file(&path)? == entry.blake3,
            "Hashfehler: {}",
            entry.path
        );
    }
    let database =
        Connection::open_with_flags(source.join("metadata.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = database.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(integrity == "ok", "SQLite integrity_check: {integrity}");
    Ok(manifest)
}

pub fn restore_backup(source: &Path, destination: &Path) -> anyhow::Result<BackupManifest> {
    let manifest = verify_backup(source)?;
    if destination.exists() {
        ensure!(destination.is_dir(), "Restore-Ziel ist kein Verzeichnis");
        ensure!(
            fs::read_dir(destination)?.next().is_none(),
            "Restore-Ziel muss leer sein"
        );
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = destination
        .file_name()
        .context("Restore-Ziel braucht einen Verzeichnisnamen")?;
    let staging = parent.join(format!(
        ".{}.restore-{}",
        name.to_string_lossy(),
        Uuid::now_v7()
    ));
    fs::create_dir(&staging)?;
    let result = (|| {
        for entry in &manifest.entries {
            let relative = safe_relative(&entry.path)?;
            let target = staging.join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source.join(&relative), target)?;
        }
        if destination.exists() {
            fs::remove_dir(destination)?;
        }
        fs::rename(&staging, destination)?;
        let database = Database::open(destination.join("metadata.db"))?;
        database.integrity_check()?;
        Ok(manifest.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn copy_tree(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if !source.exists() {
        fs::create_dir_all(destination)?;
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!("Spezialdatei im Blockspeicher: {}", entry.path().display());
        }
    }
    Ok(())
}

fn collect_entries(root: &Path) -> anyhow::Result<Vec<BackupEntry>> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<BackupEntry>) -> anyhow::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_name() == MANIFEST_NAME {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if file_type.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                output.push(BackupEntry {
                    path: relative,
                    size: entry.metadata()?.len(),
                    blake3: hash_file(&entry.path())?,
                });
            } else {
                bail!("Spezialdatei im Backup: {}", entry.path().display());
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries)?;
    Ok(entries)
}

fn safe_relative(value: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(value);
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "Unsicherer Pfad im Backup-Manifest"
    );
    Ok(path.to_path_buf())
}

fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let temporary = path.with_extension("json.partial");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}
