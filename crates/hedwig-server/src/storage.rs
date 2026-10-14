use anyhow::{Context, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct BlockStore {
    root: PathBuf,
}

impl BlockStore {
    pub async fn new(root: PathBuf) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(&root)
            .await
            .with_context(|| format!("cannot create block store {}", root.display()))?;
        Ok(Self { root })
    }

    fn block_path(&self, space_id: &str, hash: &str) -> anyhow::Result<PathBuf> {
        if !space_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("invalid block identifier");
        }
        Ok(self.root.join(space_id).join(&hash[0..2]).join(hash))
    }

    pub async fn contains(
        &self,
        space_id: &str,
        hash: &str,
        expected_size: u32,
    ) -> anyhow::Result<bool> {
        let path = self.block_path(space_id, hash)?;
        match tokio::fs::metadata(path).await {
            Ok(metadata) => Ok(metadata.is_file() && metadata.len() == u64::from(expected_size)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn put_verified(
        &self,
        space_id: &str,
        hash: &str,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let actual = blake3::hash(data).to_hex().to_string();
        if actual != hash {
            bail!("block hash mismatch");
        }
        let target = self.block_path(space_id, hash)?;
        if tokio::fs::try_exists(&target).await? {
            let existing = tokio::fs::read(&target).await?;
            if existing.len() == data.len() && blake3::hash(&existing).to_hex().as_str() == hash {
                return Ok(());
            }
            tokio::fs::remove_file(&target).await?;
        }
        let directory = target.parent().expect("block path has a parent");
        tokio::fs::create_dir_all(directory).await?;
        let temporary = directory.join(format!(".{}.upload", Uuid::new_v4()));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await?;
        if let Err(error) = file.write_all(data).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error.into());
        }
        if let Err(error) = file.flush().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error.into());
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error.into());
        }
        drop(file);
        match tokio::fs::rename(&temporary, &target).await {
            Ok(()) => Ok(()),
            Err(_error) if tokio::fs::try_exists(&target).await.unwrap_or(false) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                Ok(())
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                Err(error.into())
            }
        }
    }

    /// Durably installs many immutable blocks with two filesystem-wide barriers per filesystem:
    /// once for the complete temporary contents and once for the atomic renames. Database commits
    /// happen only after this returns. Non-Linux targets retain the conservative per-file fsync.
    pub async fn put_verified_batch(
        &self,
        blocks: Vec<(String, String, Vec<u8>)>,
    ) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let store = self.clone();
            return tokio::task::spawn_blocking(move || store.put_verified_batch_linux(blocks))
                .await
                .map_err(|error| anyhow::anyhow!("block batch task failed: {error}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            for (space_id, hash, data) in blocks {
                self.put_verified(&space_id, &hash, &data).await?;
            }
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn put_verified_batch_linux(
        &self,
        blocks: Vec<(String, String, Vec<u8>)>,
    ) -> anyhow::Result<()> {
        use std::collections::{HashMap, HashSet};
        use std::fs::OpenOptions;
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        let mut unique = HashSet::new();
        let mut pending = Vec::new();
        let mut filesystems = HashMap::new();
        for (space_id, hash, data) in blocks {
            if !unique.insert((space_id.clone(), hash.clone())) {
                continue;
            }
            if blake3::hash(&data).to_hex().as_str() != hash {
                bail!("block hash mismatch");
            }
            let target = self.block_path(&space_id, &hash)?;
            if target.exists() {
                let existing = std::fs::read(&target)?;
                if existing.len() == data.len() && blake3::hash(&existing).to_hex().as_str() == hash
                {
                    continue;
                }
                std::fs::remove_file(&target)?;
            }
            let directory = target.parent().expect("block path has a parent");
            std::fs::create_dir_all(directory)?;
            let temporary = directory.join(format!(".{}.upload", Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            if let Err(error) = file.write_all(&data).and_then(|_| file.flush()) {
                drop(file);
                let _ = std::fs::remove_file(&temporary);
                return Err(error.into());
            }
            let device = file.metadata()?.dev();
            if let std::collections::hash_map::Entry::Vacant(entry) = filesystems.entry(device) {
                entry.insert(OpenOptions::new().read(true).open(directory)?);
            }
            // syncfs works on the complete filesystem, so one open directory descriptor per
            // filesystem is sufficient. Closing each block here prevents large batches from
            // exhausting RLIMIT_NOFILE while preserving both durability barriers.
            drop(file);
            pending.push((temporary, target));
        }
        if pending.is_empty() {
            return Ok(());
        }

        for directory in filesystems.values() {
            // SAFETY: the descriptor belongs to an open directory and remains valid for the call.
            if unsafe { libc::syncfs(directory.as_raw_fd()) } != 0 {
                let error = std::io::Error::last_os_error();
                for (temporary, _) in &pending {
                    let _ = std::fs::remove_file(temporary);
                }
                return Err(error.into());
            }
        }
        for (temporary, target) in pending {
            match std::fs::rename(&temporary, &target) {
                Ok(()) => {}
                Err(_) if target.exists() => {
                    let _ = std::fs::remove_file(&temporary);
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(error.into());
                }
            }
        }
        for directory in filesystems.values() {
            // SAFETY: the descriptor belongs to an open directory and remains valid for the call.
            if unsafe { libc::syncfs(directory.as_raw_fd()) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    pub async fn read(&self, space_id: &str, hash: &str) -> anyhow::Result<Vec<u8>> {
        let path = self.block_path(space_id, hash)?;
        tokio::fs::read(&path)
            .await
            .with_context(|| format!("cannot read block {}", path.display()))
    }

    pub async fn remove(&self, space_id: &str, hash: &str) -> anyhow::Result<bool> {
        let path = self.block_path(space_id, hash)?;
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::BlockStore;

    #[tokio::test]
    async fn incomplete_existing_block_is_rejected_and_repaired() {
        let temporary = tempfile::TempDir::new().unwrap();
        let store = BlockStore::new(temporary.path().join("blocks"))
            .await
            .unwrap();
        let space_id = "space-test";
        let content = b"vollstaendiger Blockinhalt";
        let hash = blake3::hash(content).to_hex().to_string();
        let target = store.block_path(space_id, &hash).unwrap();
        tokio::fs::create_dir_all(target.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&target, &content[..5]).await.unwrap();

        assert!(
            !store
                .contains(space_id, &hash, content.len() as u32)
                .await
                .unwrap()
        );
        store.put_verified(space_id, &hash, content).await.unwrap();
        assert!(
            store
                .contains(space_id, &hash, content.len() as u32)
                .await
                .unwrap()
        );
        assert_eq!(store.read(space_id, &hash).await.unwrap(), content);
    }

    #[tokio::test]
    async fn batch_install_verifies_deduplicates_and_persists_every_block() {
        let temporary = tempfile::TempDir::new().unwrap();
        let store = BlockStore::new(temporary.path().join("blocks"))
            .await
            .unwrap();
        let space = "space-batch";
        let first = b"erster sicherer Block".to_vec();
        let second = b"zweiter sicherer Block".to_vec();
        let first_hash = blake3::hash(&first).to_hex().to_string();
        let second_hash = blake3::hash(&second).to_hex().to_string();
        store
            .put_verified_batch(vec![
                (space.into(), first_hash.clone(), first.clone()),
                (space.into(), second_hash.clone(), second.clone()),
                (space.into(), first_hash.clone(), first.clone()),
            ])
            .await
            .unwrap();

        assert_eq!(store.read(space, &first_hash).await.unwrap(), first);
        assert_eq!(store.read(space, &second_hash).await.unwrap(), second);
        let wrong_hash = blake3::hash(b"anderer Inhalt").to_hex().to_string();
        assert!(
            store
                .put_verified_batch(vec![(space.into(), wrong_hash, b"falsch".to_vec())])
                .await
                .is_err()
        );
    }
}
