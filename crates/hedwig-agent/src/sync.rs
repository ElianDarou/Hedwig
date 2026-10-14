use crate::state::{Entry, State};
use anyhow::{Context, bail};
use fs2::FileExt;
use futures::{StreamExt, stream};
use hedwig_client::{CloudClient, FileUpload, RemoteFile};
use hedwig_core::{CloudPath, IgnoreRules};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    TwoWay,
    UploadOnly,
    DownloadOnly,
}

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub space_id: String,
    pub root: PathBuf,
    pub state_dir: PathBuf,
    pub mode: SyncMode,
    pub delete_limit: usize,
    pub delete_percent: usize,
    /// Allows the currently detected deletion batch exactly for this run.
    /// Callers must require an explicit user confirmation before setting it.
    pub allow_deletions_once: bool,
    /// Cloud folders intentionally not materialized below this local root.
    /// This is separate from ignore rules and never implies a remote deletion.
    pub selective_excluded: Vec<String>,
    /// Maximum number of independent file transfers in flight. Transfers stay atomic per file.
    pub parallel_transfers: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub uploaded: usize,
    pub downloaded: usize,
    pub removed_local: usize,
    pub removed_remote: usize,
    pub renamed_remote: usize,
    pub conflicts: usize,
    pub excluded: usize,
    pub errors: Vec<String>,
    pub deletion_pause: bool,
    pub pending_deletions: usize,
    pub selective_preserved: usize,
}

#[derive(Debug, Clone)]
struct LocalFile {
    path: String,
    absolute: PathBuf,
    hash: String,
}

pub async fn synchronize(client: CloudClient, config: SyncConfig) -> anyhow::Result<SyncReport> {
    ensure_root(&config.root)?;
    std::fs::create_dir_all(&config.state_dir)?;
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(config.state_dir.join("sync.lock"))?;
    lock_file
        .try_lock_exclusive()
        .context("Für diesen Ordner läuft bereits ein Abgleich")?;
    let state = State::open(&config.state_dir.join("state.db"))?;
    let mut entries = state.entries()?;
    for entry in entries
        .iter()
        .filter(|entry| is_selectively_excluded(&config.selective_excluded, &entry.path))
    {
        state.remove(&entry.file_id)?;
    }
    entries.retain(|entry| !is_selectively_excluded(&config.selective_excluded, &entry.path));
    let legacy_shared =
        std::fs::read_to_string(config.root.join(".cloudignore")).unwrap_or_default();
    let shared = format!(
        "{legacy_shared}\n{}",
        std::fs::read_to_string(config.root.join(".hedwigignore")).unwrap_or_default()
    );
    let local = std::fs::read_to_string(config.state_dir.join("local.hedwigignore"))
        .or_else(|_| std::fs::read_to_string(config.state_dir.join("local.cloudignore")))
        .unwrap_or_default();
    let ignore = IgnoreRules::parse(&shared, &local)?;
    let (mut locals, excluded) = scan_local(&config.root, &config.state_dir, &ignore)?;
    let selective_count = locals
        .values()
        .filter(|local| is_selectively_excluded(&config.selective_excluded, &local.path))
        .count();
    for local in locals
        .values()
        .filter(|local| is_selectively_excluded(&config.selective_excluded, &local.path))
    {
        preserve_selective(local, &config.state_dir)?;
    }
    locals.retain(|path, _| !is_selectively_excluded(&config.selective_excluded, path));
    let mut remotes = client.files(&config.space_id, true).await?;
    let mut report = SyncReport {
        excluded: excluded.len() + selective_count,
        selective_preserved: selective_count,
        ..Default::default()
    };
    if config.mode == SyncMode::TwoWay {
        let tracked_paths: HashSet<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        let occupied_remote_paths: HashSet<_> = remotes
            .iter()
            .filter(|remote| remote.deleted_at.is_none())
            .map(|remote| remote.path.clone())
            .collect();
        let mut claimed = HashSet::new();
        for index in 0..entries.len() {
            let previous = entries[index].clone();
            if locals.contains_key(&previous.path) {
                continue;
            }
            let candidates: Vec<_> = locals
                .values()
                .filter(|local| {
                    local.hash == previous.local_hash
                        && !tracked_paths.contains(&local.path)
                        && !occupied_remote_paths.contains(&local.path)
                        && !claimed.contains(&local.path)
                })
                .cloned()
                .collect();
            if candidates.len() != 1 {
                continue;
            }
            let Some(remote_index) = remotes
                .iter()
                .position(|remote| remote.id == previous.file_id && remote.deleted_at.is_none())
            else {
                continue;
            };
            let candidate = &candidates[0];
            claimed.insert(candidate.path.clone());
            if remotes[remote_index].path == previous.path {
                let moved = client.move_file(&previous.file_id, &candidate.path).await?;
                entries[index].path = moved.path.clone();
                remotes[remote_index].path = moved.path;
                state.put(&entries[index])?;
                report.renamed_remote += 1;
            } else {
                let uploaded = upload_stable(&client, &config, candidate, None, None).await?;
                state.put(&Entry {
                    file_id: uploaded.file_id,
                    path: uploaded.path,
                    version_id: uploaded.version_id,
                    local_hash: candidate.hash.clone(),
                })?;
                locals.remove(&candidate.path);
                let remote = remotes[remote_index].clone();
                let target = config.root.join(CloudPath::parse(&remote.path)?.as_str());
                if target.exists() {
                    preserve_local_conflict(&target)?;
                    report.conflicts += 1;
                }
                let downloaded = download(&client, &config, &remote).await?;
                state.put(&downloaded)?;
                entries[index] = downloaded.clone();
                locals.insert(
                    downloaded.path.clone(),
                    LocalFile {
                        path: downloaded.path,
                        absolute: target,
                        hash: downloaded.local_hash,
                    },
                );
                report.conflicts += 1;
                report.downloaded += 1;
            }
        }
    }
    let remote_by_id: HashMap<_, _> = remotes.iter().map(|file| (file.id.clone(), file)).collect();
    let entry_by_id: HashMap<_, _> = entries
        .iter()
        .map(|entry| (entry.file_id.clone(), entry))
        .collect();
    let entry_by_path: HashMap<_, _> = entries
        .iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    let missing_for_remote_delete: Vec<_> = entries
        .iter()
        .filter(|entry| {
            remote_by_id
                .get(&entry.file_id)
                .is_some_and(|remote| remote.deleted_at.is_none())
                && !locals.contains_key(&entry.path)
                && !is_excluded(&ignore, &entry.path)
        })
        .collect();
    let percentage = if entries.is_empty() {
        0
    } else {
        missing_for_remote_delete.len() * 100 / entries.len()
    };
    let deletions_paused = !config.allow_deletions_once
        && config.mode == SyncMode::TwoWay
        && (missing_for_remote_delete.len() > config.delete_limit
            || percentage > config.delete_percent);
    report.deletion_pause = deletions_paused;
    report.pending_deletions = if deletions_paused {
        missing_for_remote_delete.len()
    } else {
        0
    };

    let mut new_remote_downloads = Vec::new();
    for remote in &remotes {
        if is_selectively_excluded(&config.selective_excluded, &remote.path) {
            continue;
        }
        let Some(version_id) = remote.current_version_id.as_deref() else {
            continue;
        };
        let tracked = entry_by_id.get(&remote.id).copied();
        if remote.deleted_at.is_some() {
            if let Some(entry) = tracked {
                if let Some(local) = locals.remove(&entry.path) {
                    if config.mode == SyncMode::UploadOnly {
                        continue;
                    } else if local.hash == entry.local_hash {
                        archive_deleted(&local.absolute, &config.state_dir)?;
                        report.removed_local += 1;
                        state.remove(&remote.id)?;
                    } else if config.mode != SyncMode::DownloadOnly {
                        let result = upload_stable(
                            &client,
                            &config,
                            &local,
                            Some(&remote.id),
                            Some(&entry.version_id),
                        )
                        .await?;
                        move_to_server_path(&local.absolute, &config.root, &result.path)?;
                        state.put(&Entry {
                            file_id: result.file_id,
                            path: result.path,
                            version_id: result.version_id,
                            local_hash: local.hash,
                        })?;
                        report.conflicts += 1;
                    } else {
                        preserve_local_conflict(&local.absolute)?;
                        state.remove(&remote.id)?;
                        report.conflicts += 1;
                    }
                } else {
                    state.remove(&remote.id)?;
                }
            }
            continue;
        }

        match tracked {
            Some(entry) => {
                let mut local = locals.remove(&entry.path);
                if config.mode != SyncMode::UploadOnly
                    && remote.path != entry.path
                    && let Some(current) = local.as_mut()
                    && current.hash == entry.local_hash
                {
                    let target = config.root.join(CloudPath::parse(&remote.path)?.as_str());
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    if target.exists() {
                        preserve_local_conflict(&target)?;
                        report.conflicts += 1;
                    }
                    std::fs::rename(&current.absolute, &target)?;
                    current.path = remote.path.clone();
                    current.absolute = target;
                }
                match local {
                    Some(local)
                        if remote.current_version_id.as_deref()
                            == Some(entry.version_id.as_str()) =>
                    {
                        if local.hash != entry.local_hash && config.mode != SyncMode::DownloadOnly {
                            let result = upload_stable(
                                &client,
                                &config,
                                &local,
                                Some(&remote.id),
                                Some(&entry.version_id),
                            )
                            .await?;
                            move_to_server_path(&local.absolute, &config.root, &result.path)?;
                            state.put(&Entry {
                                file_id: result.file_id,
                                path: result.path,
                                version_id: result.version_id,
                                local_hash: local.hash,
                            })?;
                            report.uploaded += 1;
                        } else if remote.path != entry.path {
                            state.put(&Entry {
                                path: remote.path.clone(),
                                ..entry.clone()
                            })?;
                        }
                    }
                    Some(local) => {
                        if local.hash != entry.local_hash && config.mode != SyncMode::DownloadOnly {
                            let result = upload_stable(
                                &client,
                                &config,
                                &local,
                                Some(&remote.id),
                                Some(&entry.version_id),
                            )
                            .await?;
                            if result.conflict {
                                move_to_server_path(&local.absolute, &config.root, &result.path)?;
                                state.put(&Entry {
                                    file_id: result.file_id,
                                    path: result.path,
                                    version_id: result.version_id,
                                    local_hash: local.hash.clone(),
                                })?;
                                report.conflicts += 1;
                            }
                        }
                        if config.mode == SyncMode::DownloadOnly {
                            if local.hash != entry.local_hash {
                                preserve_local_conflict(&local.absolute)?;
                                report.conflicts += 1;
                            }
                            let entry = download(&client, &config, remote).await?;
                            state.put(&entry)?;
                            report.downloaded += 1;
                        } else if config.mode == SyncMode::TwoWay {
                            let entry = download(&client, &config, remote).await?;
                            state.put(&entry)?;
                            report.downloaded += 1;
                        }
                    }
                    None if config.mode == SyncMode::DownloadOnly => {
                        let entry = download(&client, &config, remote).await?;
                        state.put(&entry)?;
                        report.downloaded += 1;
                    }
                    None if config.mode == SyncMode::TwoWay && !deletions_paused => {
                        client.delete_file(&remote.id).await?;
                        state.remove(&remote.id)?;
                        report.removed_remote += 1;
                    }
                    None => {}
                }
            }
            None if config.mode != SyncMode::UploadOnly => {
                if let Some(local) = locals.remove(&remote.path) {
                    if config.mode == SyncMode::TwoWay {
                        let result =
                            upload_stable(&client, &config, &local, Some(&remote.id), None).await?;
                        move_to_server_path(&local.absolute, &config.root, &result.path)?;
                        state.put(&Entry {
                            file_id: result.file_id,
                            path: result.path,
                            version_id: result.version_id,
                            local_hash: local.hash,
                        })?;
                        report.conflicts += 1;
                    } else {
                        preserve_local_conflict(&local.absolute)?;
                        report.conflicts += 1;
                    }
                }
                new_remote_downloads.push(remote.clone());
            }
            None => {}
        }
        let _ = version_id;
    }

    if !new_remote_downloads.is_empty() {
        let parallel = client.transfer_parallelism(config.parallel_transfers);
        let transfers = stream::iter(new_remote_downloads.into_iter().map(|remote| {
            let client = client.clone();
            let config = config.clone();
            async move { download(&client, &config, &remote).await }
        }))
        .buffer_unordered(parallel);
        futures::pin_mut!(transfers);
        let mut first_error = None;
        let mut completed = Vec::new();
        while let Some(result) = transfers.next().await {
            match result {
                Ok(entry) => {
                    report.downloaded += 1;
                    completed.push(entry);
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        state.put_many(&completed)?;
        if let Some(error) = first_error {
            return Err(error);
        }
    }

    if config.mode != SyncMode::DownloadOnly {
        let pending: Vec<_> = locals
            .into_values()
            .filter(|local| !entry_by_path.contains_key(&local.path))
            .collect();
        for batch in pending.chunks(1_024) {
            let uploads = batch
                .iter()
                .map(|local| FileUpload {
                    remote_path: local.path.clone(),
                    file_id: None,
                    base_version_id: None,
                    local_path: local.absolute.clone(),
                    expected_hash: local.hash.clone(),
                    idempotency_key: Uuid::new_v4().to_string(),
                })
                .collect();
            let results = client.upload_files(&config.space_id, uploads).await?;
            if results.len() != batch.len() {
                bail!("Server lieferte nicht für jede Datei ein Upload-Ergebnis");
            }
            let mut completed = Vec::with_capacity(batch.len());
            for (local, result) in batch.iter().zip(results) {
                move_to_server_path(&local.absolute, &config.root, &result.path)?;
                completed.push(Entry {
                    file_id: result.file_id,
                    path: result.path,
                    version_id: result.version_id,
                    local_hash: local.hash.clone(),
                });
                report.uploaded += 1;
            }
            state.put_many(&completed)?;
        }
    }
    lock_file.unlock()?;
    Ok(report)
}

fn ensure_root(root: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(root).with_context(|| {
        format!(
            "Sync-Ordner {} ist nicht erreichbar; es werden keine Löschungen ausgeführt",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        bail!("Sync-Pfad ist kein Ordner");
    }
    Ok(())
}

fn scan_local(
    root: &Path,
    state_dir: &Path,
    ignore: &IgnoreRules,
) -> anyhow::Result<(HashMap<String, LocalFile>, HashSet<String>)> {
    let mut candidates = Vec::new();
    let mut excluded = HashSet::new();
    for item in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if item.path() == root {
            continue;
        }
        if item.path().starts_with(state_dir) {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let decision = ignore.evaluate(Path::new(&relative), item.file_type().is_dir());
        if (relative.ends_with(".hedwig-partial") || relative.ends_with(".cloud-partial"))
            || (decision.ignored && relative != ".hedwigignore" && relative != ".cloudignore")
        {
            excluded.insert(relative);
            continue;
        }
        if item.file_type().is_symlink() {
            excluded.insert(relative);
            continue;
        }
        if item.file_type().is_file() {
            candidates.push((relative, item.path().to_path_buf()));
        }
    }
    // Hashing is independent per file. Parallel workers shorten full safety scans without
    // weakening them: every file is still read completely and checked for changes during read.
    let hashed: Vec<anyhow::Result<LocalFile>> = candidates
        .into_par_iter()
        .map(|(path, absolute)| {
            let hash = hash_stable(&absolute)?;
            Ok(LocalFile {
                path,
                absolute,
                hash,
            })
        })
        .collect();
    let mut files = HashMap::with_capacity(hashed.len());
    for local in hashed {
        let local = local?;
        files.insert(local.path.clone(), local);
    }
    Ok((files, excluded))
}

fn hash_stable(path: &Path) -> anyhow::Result<String> {
    let before = std::fs::metadata(path)?;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut hasher = blake3::Hasher::new();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = std::fs::metadata(path)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        bail!(
            "Datei {} wurde während des Einlesens geändert",
            path.display()
        );
    }
    Ok(hasher.finalize().to_hex().to_string())
}

async fn upload_stable(
    client: &CloudClient,
    config: &SyncConfig,
    local: &LocalFile,
    file_id: Option<&str>,
    base: Option<&str>,
) -> anyhow::Result<hedwig_client::CommitResult> {
    Ok(client
        .upload_file(
            &config.space_id,
            &local.path,
            file_id,
            base,
            &local.absolute,
            &local.hash,
            &Uuid::new_v4().to_string(),
        )
        .await?)
}

async fn download(
    client: &CloudClient,
    config: &SyncConfig,
    remote: &RemoteFile,
) -> anyhow::Result<Entry> {
    let relative = CloudPath::parse(&remote.path)?;
    let target = config.root.join(relative.as_str());
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = target.with_file_name(format!(".{}.hedwig-partial", Uuid::new_v4()));
    let result = client.download_verified_to(&remote.id, &temporary).await;
    let (manifest, local_hash) = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error.into());
        }
    };
    tokio::fs::rename(&temporary, &target).await?;
    sync_parent_directory(&target).await?;
    Ok(Entry {
        file_id: remote.id.clone(),
        path: manifest.path,
        version_id: manifest.version_id,
        local_hash,
    })
}

async fn sync_parent_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .context("Datei hat keinen Zielordner")?
            .to_owned();
        tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
            .await
            .map_err(|error| anyhow::anyhow!("Ordner-Sync wurde abgebrochen: {error}"))??;
    }
    Ok(())
}

fn archive_deleted(path: &Path, state_dir: &Path) -> anyhow::Result<()> {
    let archive = state_dir.join("removed").join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(archive.parent().unwrap())?;
    std::fs::rename(path, archive)?;
    Ok(())
}

fn preserve_selective(local: &LocalFile, state_dir: &Path) -> anyhow::Result<()> {
    let target = state_dir
        .join("selective-preserved")
        .join(Uuid::now_v7().to_string())
        .join(CloudPath::parse(&local.path)?.as_str());
    std::fs::create_dir_all(target.parent().context("Zielordner fehlt")?)?;
    match std::fs::rename(&local.absolute, &target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(18) => {
            std::fs::copy(&local.absolute, &target)?;
            std::fs::File::open(&target)?.sync_all()?;
            std::fs::remove_file(&local.absolute)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
fn preserve_local_conflict(path: &Path) -> anyhow::Result<PathBuf> {
    let parent = path
        .parent()
        .context("Datei hat keinen übergeordneten Ordner")?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Datei");
    let (stem, extension) = name
        .rsplit_once('.')
        .map_or((name, ""), |(left, right)| (left, right));
    for number in 1..=10_000 {
        let candidate = if extension.is_empty() {
            parent.join(format!("{stem} (Lokaler Konflikt {number})"))
        } else {
            parent.join(format!("{stem} (Lokaler Konflikt {number}).{extension}"))
        };
        if !candidate.exists() {
            std::fs::rename(path, &candidate)?;
            return Ok(candidate);
        }
    }
    bail!("Kein freier Name für lokale Konfliktkopie")
}
fn move_to_server_path(source: &Path, root: &Path, path: &str) -> anyhow::Result<()> {
    let target = root.join(CloudPath::parse(path)?.as_str());
    if source == target {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(source, target)?;
    Ok(())
}
fn is_excluded(ignore: &IgnoreRules, path: &str) -> bool {
    ignore.evaluate(Path::new(path), false).ignored
}

fn is_selectively_excluded(prefixes: &[String], path: &str) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
}
