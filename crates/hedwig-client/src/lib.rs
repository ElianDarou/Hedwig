use bytes::Bytes;
use futures::{StreamExt, stream};
use hedwig_core::{ChunkDescriptor, ChunkFormat, chunk_bytes};
use rayon::prelude::*;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{path::Path, time::Duration};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use url::Url;

const PROTOCOL_HEADER: &str = "x-hedwig-protocol";
const PROTOCOL_VERSION: &str = "1";

#[derive(Clone)]
pub struct CloudClient {
    http: reqwest::Client,
    base_url: Url,
    token: String,
    upload_bytes_per_second: u64,
    download_bytes_per_second: u64,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Serveradresse ist ungültig: {0}")]
    InvalidServer(#[from] url::ParseError),
    #[error("Serveranfrage fehlgeschlagen: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Serverfehler {status}: {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("Serverantwort konnte nicht gelesen werden: {0}")]
    InvalidResponse(#[from] serde_json::Error),
    #[error("Heruntergeladener Inhalt ist beschädigt: Block {0} stimmt nicht überein")]
    CorruptDownload(String),
    #[error("Datei hat sich während des Einlesens geändert")]
    ChangedWhileReading,
    #[error("Lokale Datei konnte nicht geschrieben werden: {0}")]
    Io(#[from] std::io::Error),
}

impl ClientError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(
            self,
            Self::Api {
                status: StatusCode::UNAUTHORIZED,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub personal_space_id: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteFile {
    pub id: String,
    pub path: String,
    pub size: i64,
    pub current_version_id: Option<String>,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDescriptor {
    pub hash: String,
    pub size: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileManifest {
    pub file_id: String,
    pub version_id: String,
    pub path: String,
    pub size: i64,
    pub blocks: Vec<BlockDescriptor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitResult {
    pub file_id: String,
    pub version_id: String,
    pub path: String,
    pub conflict: bool,
    pub change_sequence: i64,
}

#[derive(Debug, Clone)]
pub struct FileUpload {
    pub remote_path: String,
    pub file_id: Option<String>,
    pub base_version_id: Option<String>,
    pub local_path: std::path::PathBuf,
    pub expected_hash: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MutationResult {
    pub file_id: String,
    pub path: String,
    pub change_sequence: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Change {
    pub sequence: i64,
    pub event_id: String,
    pub space_id: String,
    pub object_id: String,
    pub kind: String,
    pub path: String,
    pub version_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
struct SpacesResponse {
    spaces: Vec<Space>,
}
#[derive(Debug, Deserialize)]
struct FilesResponse {
    files: Vec<RemoteFile>,
}
#[derive(Debug, Deserialize)]
struct ChangesResponse {
    changes: Vec<Change>,
    next_cursor: i64,
}
#[derive(Debug, Deserialize)]
struct WaitForChangeResponse {
    changed: bool,
}
#[derive(Debug, Deserialize)]
struct StartUploadResponse {
    upload_id: String,
    missing_blocks: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct StartUploadBatchResponse {
    uploads: Vec<StartUploadResponse>,
}
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}
#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Clone)]
pub struct ChangePage {
    pub changes: Vec<Change>,
    pub next_cursor: i64,
}

impl CloudClient {
    pub async fn login(
        server: &str,
        username: &str,
        password: &str,
        device_name: &str,
    ) -> Result<(Self, Session), ClientError> {
        Self::login_with_otp(server, username, password, device_name, None).await
    }

    pub async fn login_with_otp(
        server: &str,
        username: &str,
        password: &str,
        device_name: &str,
        otp: Option<&str>,
    ) -> Result<(Self, Session), ClientError> {
        #[derive(Serialize)]
        struct LoginRequest<'a> {
            username: &'a str,
            password: &'a str,
            device_name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            otp: Option<&'a str>,
        }
        let mut base_url = Url::parse(server)?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let http = build_http()?;
        let response = http
            .post(base_url.join("v1/auth/login")?)
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION)
            .json(&LoginRequest {
                username,
                password,
                device_name,
                otp,
            })
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(api_error(status, &bytes));
        }
        let session: Session = serde_json::from_slice(&bytes)?;
        let client = Self {
            http,
            base_url,
            token: session.token.clone(),
            upload_bytes_per_second: 0,
            download_bytes_per_second: 0,
        };
        Ok((client, session))
    }

    pub fn new(server: &str, token: impl Into<String>) -> Result<Self, ClientError> {
        let mut base_url = Url::parse(server)?;
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let http = build_http()?;
        Ok(Self {
            http,
            base_url,
            token: token.into(),
            upload_bytes_per_second: 0,
            download_bytes_per_second: 0,
        })
    }

    /// Limits payload traffic for this client. A value of zero means unlimited.
    pub fn with_transfer_limits(mut self, upload: u64, download: u64) -> Self {
        self.upload_bytes_per_second = upload;
        self.download_bytes_per_second = download;
        self
    }

    /// Bandwidth limits are tokenized per request today, so parallel requests would multiply a
    /// configured limit. Until the shared limiter is introduced, limited clients deliberately run
    /// one transfer at a time; unlimited clients can use the requested safe concurrency.
    pub fn transfer_parallelism(&self, requested: usize) -> usize {
        if self.upload_bytes_per_second > 0 || self.download_bytes_per_second > 0 {
            1
        } else {
            requested.clamp(1, 64)
        }
    }

    pub async fn spaces(&self) -> Result<Vec<Space>, ClientError> {
        let response: SpacesResponse = self
            .json(Method::GET, "v1/spaces", Option::<&()>::None)
            .await?;
        Ok(response.spaces)
    }

    /// Invalidates the current server session. Callers should only discard the locally stored
    /// token after this request succeeded (or the server reports that it is already invalid).
    pub async fn logout(&self) -> Result<(), ClientError> {
        self.bytes(Method::POST, "v1/auth/logout", None).await?;
        Ok(())
    }

    pub async fn files(
        &self,
        space_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<RemoteFile>, ClientError> {
        let path = format!(
            "v1/spaces/{space_id}/files{}",
            if include_deleted {
                "?include_deleted=true"
            } else {
                ""
            }
        );
        let response: FilesResponse = self.json(Method::GET, &path, Option::<&()>::None).await?;
        Ok(response.files)
    }

    pub async fn changes(&self, after: i64, limit: usize) -> Result<ChangePage, ClientError> {
        let response: ChangesResponse = self
            .json(
                Method::GET,
                &format!("v1/changes?after={after}&limit={}", limit.min(1000)),
                Option::<&()>::None,
            )
            .await?;
        Ok(ChangePage {
            changes: response.changes,
            next_cursor: response.next_cursor,
        })
    }

    /// Waits for a content-free server wake-up signal. The durable change log remains the
    /// source of truth; a timeout or lost signal is therefore safe and only delays a sync.
    pub async fn wait_for_change(&self, timeout: Duration) -> Result<bool, ClientError> {
        let seconds = timeout.as_secs().clamp(1, 30);
        let response: WaitForChangeResponse = self
            .json(
                Method::GET,
                &format!("v1/events/wait?timeout_seconds={seconds}"),
                Option::<&()>::None,
            )
            .await?;
        Ok(response.changed)
    }

    pub async fn manifest(&self, file_id: &str) -> Result<FileManifest, ClientError> {
        self.json(
            Method::GET,
            &format!("v1/files/{file_id}/manifest"),
            Option::<&()>::None,
        )
        .await
    }

    pub async fn download_verified(
        &self,
        file_id: &str,
    ) -> Result<(FileManifest, Vec<u8>), ClientError> {
        let manifest = self.manifest(file_id).await?;
        let bytes = self
            .bytes(Method::GET, &format!("v1/files/{file_id}/content"), None)
            .await?;
        let expected_size: usize = manifest
            .blocks
            .iter()
            .map(|block| block.size as usize)
            .sum();
        if bytes.len() != expected_size {
            return Err(ClientError::CorruptDownload("Dateigröße".into()));
        }
        let mut offset = 0;
        for block in &manifest.blocks {
            let end = offset + block.size as usize;
            let actual = blake3::hash(&bytes[offset..end]).to_hex().to_string();
            if actual != block.hash {
                return Err(ClientError::CorruptDownload(block.hash.clone()));
            }
            offset = end;
        }
        Ok((manifest, bytes.to_vec()))
    }

    /// Streams a verified download into a new local file and returns the whole-file hash.
    /// Verification follows the immutable block manifest, so corrupt bytes never become a
    /// confirmed local file.
    pub async fn download_verified_to(
        &self,
        file_id: &str,
        target: &Path,
    ) -> Result<(FileManifest, String), ClientError> {
        let manifest = self.manifest(file_id).await?;
        let response = self
            .http
            .get(self.url(&format!("v1/files/{file_id}/content"))?)
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION)
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await?;
            return Err(api_error(status, &bytes));
        }
        let mut response = response;
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target)
            .await?;
        let expected_size: usize = manifest
            .blocks
            .iter()
            .map(|block| block.size as usize)
            .sum();
        let mut total = 0usize;
        let mut block_index = 0usize;
        let mut block_bytes = 0usize;
        let mut block_hasher = blake3::Hasher::new();
        let mut whole_hasher = blake3::Hasher::new();
        while let Some(chunk) = response.chunk().await? {
            total = total.saturating_add(chunk.len());
            if total > expected_size {
                return Err(ClientError::CorruptDownload("Dateigröße".into()));
            }
            file.write_all(&chunk).await?;
            whole_hasher.update(&chunk);
            let mut offset = 0usize;
            while offset < chunk.len() {
                let Some(descriptor) = manifest.blocks.get(block_index) else {
                    return Err(ClientError::CorruptDownload("Dateigröße".into()));
                };
                let remaining = descriptor.size as usize - block_bytes;
                let take = remaining.min(chunk.len() - offset);
                block_hasher.update(&chunk[offset..offset + take]);
                block_bytes += take;
                offset += take;
                if block_bytes == descriptor.size as usize {
                    if block_hasher.finalize().to_hex().as_str() != descriptor.hash {
                        return Err(ClientError::CorruptDownload(descriptor.hash.clone()));
                    }
                    block_index += 1;
                    block_bytes = 0;
                    block_hasher = blake3::Hasher::new();
                }
            }
            throttle(chunk.len(), self.download_bytes_per_second).await;
        }
        if total != expected_size || block_index != manifest.blocks.len() || block_bytes != 0 {
            return Err(ClientError::CorruptDownload("Dateigröße".into()));
        }
        file.sync_all().await?;
        Ok((manifest, whole_hasher.finalize().to_hex().to_string()))
    }

    pub async fn upload_bytes(
        &self,
        space_id: &str,
        path: &str,
        file_id: Option<&str>,
        base_version_id: Option<&str>,
        data: &[u8],
        idempotency_key: &str,
    ) -> Result<CommitResult, ClientError> {
        let chunks = chunk_bytes(data, ChunkFormat::V1);
        let descriptors: Vec<BlockDescriptor> = chunks
            .iter()
            .map(|chunk| BlockDescriptor {
                hash: chunk.descriptor.hash.clone(),
                size: chunk.descriptor.size,
            })
            .collect();
        #[derive(Serialize)]
        struct Start<'a> {
            path: &'a str,
            file_id: Option<&'a str>,
            base_version_id: Option<&'a str>,
            size: usize,
            blocks: &'a [BlockDescriptor],
            idempotency_key: &'a str,
        }
        let started: StartUploadResponse = self
            .json(
                Method::POST,
                &format!("v1/spaces/{space_id}/uploads"),
                Some(&Start {
                    path,
                    file_id,
                    base_version_id,
                    size: data.len(),
                    blocks: &descriptors,
                    idempotency_key,
                }),
            )
            .await?;
        let missing: std::collections::HashSet<_> =
            started.missing_blocks.iter().map(String::as_str).collect();
        for chunk in chunks {
            if missing.contains(chunk.descriptor.hash.as_str()) {
                throttle(chunk.data.len(), self.upload_bytes_per_second).await;
                self.bytes(
                    Method::PUT,
                    &format!(
                        "v1/uploads/{}/blocks/{}",
                        started.upload_id, chunk.descriptor.hash
                    ),
                    Some(Bytes::copy_from_slice(chunk.data)),
                )
                .await?;
            }
        }
        self.json(
            Method::POST,
            &format!("v1/uploads/{}/commit", started.upload_id),
            Option::<&()>::None,
        )
        .await
    }

    /// Uploads a file with bounded memory. The file is streamed once to build the complete
    /// FastCDC manifest and a second time to send only blocks the server does not have.
    /// Metadata and hashes are checked across both passes before the upload is committed.
    pub async fn upload_file(
        &self,
        space_id: &str,
        remote_path: &str,
        file_id: Option<&str>,
        base_version_id: Option<&str>,
        local_path: &Path,
        expected_hash: &str,
        idempotency_key: &str,
    ) -> Result<CommitResult, ClientError> {
        use fastcdc::v2020::StreamCDC;

        let before = std::fs::metadata(local_path)?;
        let mut whole_hasher = blake3::Hasher::new();
        let mut descriptors = Vec::new();
        let mut offsets = Vec::new();
        let mut total_size = 0_u64;
        for result in StreamCDC::new(
            std::fs::File::open(local_path)?,
            hedwig_core::MIN_CHUNK_SIZE,
            hedwig_core::AVG_CHUNK_SIZE,
            hedwig_core::MAX_CHUNK_SIZE,
        ) {
            let chunk = result.map_err(std::io::Error::from)?;
            whole_hasher.update(&chunk.data);
            offsets.push(total_size);
            total_size = total_size.saturating_add(chunk.data.len() as u64);
            descriptors.push(BlockDescriptor {
                hash: blake3::hash(&chunk.data).to_hex().to_string(),
                size: chunk.data.len() as u32,
            });
        }
        let after_manifest = std::fs::metadata(local_path)?;
        if !same_file_snapshot(&before, &after_manifest)
            || whole_hasher.finalize().to_hex().as_str() != expected_hash
        {
            return Err(ClientError::ChangedWhileReading);
        }

        #[derive(Serialize)]
        struct Start<'a> {
            path: &'a str,
            file_id: Option<&'a str>,
            base_version_id: Option<&'a str>,
            size: u64,
            blocks: &'a [BlockDescriptor],
            idempotency_key: &'a str,
        }
        let started: StartUploadResponse = self
            .json(
                Method::POST,
                &format!("v1/spaces/{space_id}/uploads"),
                Some(&Start {
                    path: remote_path,
                    file_id,
                    base_version_id,
                    size: total_size,
                    blocks: &descriptors,
                    idempotency_key,
                }),
            )
            .await?;
        let missing: std::collections::HashSet<_> =
            started.missing_blocks.iter().cloned().collect();
        let before_upload = std::fs::metadata(local_path)?;
        let jobs: Vec<_> = descriptors
            .iter()
            .cloned()
            .zip(offsets)
            .filter(|(descriptor, _)| missing.contains(&descriptor.hash))
            .collect();
        let parallel = if self.upload_bytes_per_second == 0 {
            8
        } else {
            1
        };
        let transfers = stream::iter(jobs.into_iter().map(|(descriptor, offset)| {
            let client = self.clone();
            let path = local_path.to_owned();
            let upload_id = started.upload_id.clone();
            async move {
                let expected = descriptor.clone();
                let data = tokio::task::spawn_blocking(move || {
                    read_verified_range(&path, offset, &expected)
                })
                .await
                .map_err(|error| {
                    ClientError::Io(std::io::Error::other(format!(
                        "Block-Leseaufgabe wurde abgebrochen: {error}"
                    )))
                })??;
                throttle(data.len(), client.upload_bytes_per_second).await;
                client
                    .bytes(
                        Method::PUT,
                        &format!("v1/uploads/{upload_id}/blocks/{}", descriptor.hash),
                        Some(Bytes::from(data)),
                    )
                    .await?;
                Ok::<(), ClientError>(())
            }
        }))
        .buffer_unordered(parallel);
        futures::pin_mut!(transfers);
        let mut first_error = None;
        while let Some(result) = transfers.next().await {
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        let after_upload = std::fs::metadata(local_path)?;
        if !same_file_snapshot(&before_upload, &after_upload)
            || hash_file(local_path)? != expected_hash
        {
            return Err(ClientError::ChangedWhileReading);
        }

        self.json(
            Method::POST,
            &format!("v1/uploads/{}/commit", started.upload_id),
            Option::<&()>::None,
        )
        .await
    }

    /// Uploads up to 1,024 files while sharing the server's durable manifest and commit barriers.
    /// Every file is still content-defined chunked, block-verified, re-hashed after transfer, and
    /// becomes visible only in the final atomic database transaction.
    pub async fn upload_files(
        &self,
        space_id: &str,
        uploads: Vec<FileUpload>,
    ) -> Result<Vec<CommitResult>, ClientError> {
        if uploads.is_empty() {
            return Ok(Vec::new());
        }
        if uploads.len() > 1_024 {
            return Err(ClientError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Ein Upload-Batch darf höchstens 1.024 Dateien enthalten",
            )));
        }
        // FastCDC and BLAKE3 are CPU-heavy for large trees. Preparing independent files in
        // parallel shortens the time before the first byte is sent without relaxing any of the
        // snapshot, per-block hash, or final whole-file verification barriers below.
        let prepared = tokio::task::spawn_blocking(move || {
            uploads
                .into_par_iter()
                .map(prepare_file_upload)
                .collect::<Result<Vec<_>, ClientError>>()
        })
        .await
        .map_err(|error| {
            ClientError::Io(std::io::Error::other(format!(
                "Manifest-Aufgabe wurde abgebrochen: {error}"
            )))
        })??;

        #[derive(Serialize)]
        struct StartUpload<'a> {
            path: &'a str,
            file_id: Option<&'a str>,
            base_version_id: Option<&'a str>,
            size: u64,
            blocks: &'a [BlockDescriptor],
            idempotency_key: &'a str,
        }
        #[derive(Serialize)]
        struct StartBatch<'a> {
            uploads: Vec<StartUpload<'a>>,
        }
        let request = StartBatch {
            uploads: prepared
                .iter()
                .map(|item| StartUpload {
                    path: &item.upload.remote_path,
                    file_id: item.upload.file_id.as_deref(),
                    base_version_id: item.upload.base_version_id.as_deref(),
                    size: item.size,
                    blocks: &item.descriptors,
                    idempotency_key: &item.upload.idempotency_key,
                })
                .collect(),
        };
        let started: StartUploadBatchResponse = self
            .json(
                Method::POST,
                &format!("v1/spaces/{space_id}/uploads/batch"),
                Some(&request),
            )
            .await?;
        if started.uploads.len() != prepared.len() {
            return Err(ClientError::Io(std::io::Error::other(
                "Server lieferte eine unvollständige Upload-Batch-Antwort",
            )));
        }

        let mut block_jobs = Vec::new();
        for (item, started) in prepared.iter().zip(&started.uploads) {
            let missing: std::collections::HashSet<_> =
                started.missing_blocks.iter().map(String::as_str).collect();
            for (descriptor, offset) in item.descriptors.iter().cloned().zip(&item.offsets) {
                if missing.contains(descriptor.hash.as_str()) {
                    block_jobs.push((
                        started.upload_id.clone(),
                        item.upload.local_path.clone(),
                        descriptor,
                        *offset,
                    ));
                }
            }
        }
        let mut block_batches = Vec::new();
        let mut current = Vec::new();
        let mut current_bytes = 0_usize;
        for job in block_jobs {
            let encoded_size = 2 + job.0.len() + 64 + 4 + job.2.size as usize;
            if !current.is_empty()
                && (current.len() == 512 || current_bytes + encoded_size > 32 * 1024 * 1024)
            {
                block_batches.push(std::mem::take(&mut current));
                current_bytes = 0;
            }
            current_bytes += encoded_size;
            current.push(job);
        }
        if !current.is_empty() {
            block_batches.push(current);
        }
        let parallel = if self.upload_bytes_per_second == 0 {
            4
        } else {
            1
        };
        let transfers = stream::iter(block_batches.into_iter().map(|batch| {
            let client = self.clone();
            async move {
                let encoded = tokio::task::spawn_blocking(move || encode_block_batch(batch))
                    .await
                    .map_err(|error| {
                        ClientError::Io(std::io::Error::other(format!(
                            "Block-Batch-Aufgabe wurde abgebrochen: {error}"
                        )))
                    })??;
                throttle(encoded.payload.len(), client.upload_bytes_per_second).await;
                client
                    .bytes_with_content_length(
                        Method::PUT,
                        "v1/uploads/batch/blocks",
                        Bytes::from(encoded.payload),
                        encoded.content_bytes,
                    )
                    .await?;
                Ok::<(), ClientError>(())
            }
        }))
        .buffer_unordered(parallel);
        futures::pin_mut!(transfers);
        let mut first_error = None;
        while let Some(result) = transfers.next().await {
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }

        for item in &prepared {
            let after = std::fs::metadata(&item.upload.local_path)?;
            if !same_file_snapshot(&item.snapshot, &after)
                || hash_file(&item.upload.local_path)? != item.upload.expected_hash
            {
                return Err(ClientError::ChangedWhileReading);
            }
        }
        #[derive(Serialize)]
        struct CommitBatch<'a> {
            upload_ids: Vec<&'a str>,
        }
        let commit = CommitBatch {
            upload_ids: started
                .uploads
                .iter()
                .map(|upload| upload.upload_id.as_str())
                .collect(),
        };
        self.json(Method::POST, "v1/uploads/batch/commit", Some(&commit))
            .await
    }

    pub async fn delete_file(&self, file_id: &str) -> Result<(), ClientError> {
        self.bytes(Method::DELETE, &format!("v1/files/{file_id}"), None)
            .await?;
        Ok(())
    }

    pub async fn move_file(
        &self,
        file_id: &str,
        path: &str,
    ) -> Result<MutationResult, ClientError> {
        #[derive(Serialize)]
        struct MoveRequest<'a> {
            path: &'a str,
        }
        self.json(
            Method::POST,
            &format!("v1/files/{file_id}/move"),
            Some(&MoveRequest { path }),
        )
        .await
    }

    async fn json<T: DeserializeOwned, B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ClientError> {
        let mut request = self
            .http
            .request(method, self.url(path)?)
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION)
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(api_error(status, &bytes));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn bytes(
        &self,
        method: Method,
        path: &str,
        body: Option<Bytes>,
    ) -> Result<Bytes, ClientError> {
        let mut request = self
            .http
            .request(method, self.url(path)?)
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION)
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/octet-stream")
                .body(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(api_error(status, &bytes));
        }
        Ok(bytes)
    }

    async fn bytes_with_content_length(
        &self,
        method: Method,
        path: &str,
        body: Bytes,
        content_bytes: u64,
    ) -> Result<Bytes, ClientError> {
        let response = self
            .http
            .request(method, self.url(path)?)
            .header(PROTOCOL_HEADER, PROTOCOL_VERSION)
            .header("content-type", "application/octet-stream")
            .header("x-hedwig-content-bytes", content_bytes)
            .bearer_auth(&self.token)
            .body(body)
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(api_error(status, &bytes));
        }
        Ok(bytes)
    }

    fn url(&self, path: &str) -> Result<Url, ClientError> {
        Ok(self.base_url.join(path)?)
    }
}

async fn throttle(bytes: usize, bytes_per_second: u64) {
    if bytes_per_second == 0 || bytes == 0 {
        return;
    }
    tokio::time::sleep(Duration::from_secs_f64(
        bytes as f64 / bytes_per_second as f64,
    ))
    .await;
}

struct PreparedFileUpload {
    upload: FileUpload,
    descriptors: Vec<BlockDescriptor>,
    offsets: Vec<u64>,
    size: u64,
    snapshot: std::fs::Metadata,
}

fn prepare_file_upload(upload: FileUpload) -> Result<PreparedFileUpload, ClientError> {
    use fastcdc::v2020::StreamCDC;
    let before = std::fs::metadata(&upload.local_path)?;
    let mut whole_hasher = blake3::Hasher::new();
    let mut descriptors = Vec::new();
    let mut offsets = Vec::new();
    let mut size = 0_u64;
    for result in StreamCDC::new(
        std::fs::File::open(&upload.local_path)?,
        hedwig_core::MIN_CHUNK_SIZE,
        hedwig_core::AVG_CHUNK_SIZE,
        hedwig_core::MAX_CHUNK_SIZE,
    ) {
        let chunk = result.map_err(std::io::Error::from)?;
        whole_hasher.update(&chunk.data);
        offsets.push(size);
        size = size.saturating_add(chunk.data.len() as u64);
        descriptors.push(BlockDescriptor {
            hash: blake3::hash(&chunk.data).to_hex().to_string(),
            size: chunk.data.len() as u32,
        });
    }
    let snapshot = std::fs::metadata(&upload.local_path)?;
    if !same_file_snapshot(&before, &snapshot)
        || whole_hasher.finalize().to_hex().as_str() != upload.expected_hash
    {
        return Err(ClientError::ChangedWhileReading);
    }
    Ok(PreparedFileUpload {
        upload,
        descriptors,
        offsets,
        size,
        snapshot,
    })
}

fn same_file_snapshot(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    before.len() == after.len() && before.modified().ok() == after.modified().ok()
}

fn hash_file(path: &Path) -> Result<String, ClientError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn read_verified_range(
    path: &Path,
    offset: u64,
    descriptor: &BlockDescriptor,
) -> Result<Vec<u8>, ClientError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0_u8; descriptor.size as usize];
    file.read_exact(&mut data)?;
    if blake3::hash(&data).to_hex().as_str() != descriptor.hash {
        return Err(ClientError::ChangedWhileReading);
    }
    Ok(data)
}

struct EncodedBlockBatch {
    payload: Vec<u8>,
    content_bytes: u64,
}

fn encode_block_batch(
    jobs: Vec<(String, std::path::PathBuf, BlockDescriptor, u64)>,
) -> Result<EncodedBlockBatch, ClientError> {
    let count = u32::try_from(jobs.len()).map_err(|_| {
        ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Zu viele Blöcke im Upload-Batch",
        ))
    })?;
    let capacity = 8 + jobs
        .iter()
        .map(|(upload_id, _, descriptor, _)| {
            2 + upload_id.len() + 64 + 4 + descriptor.size as usize
        })
        .sum::<usize>();
    let mut payload = Vec::with_capacity(capacity);
    let mut content_bytes = 0_u64;
    payload.extend_from_slice(b"HDB1");
    payload.extend_from_slice(&count.to_be_bytes());
    for (upload_id, path, descriptor, offset) in jobs {
        let upload_length = u16::try_from(upload_id.len()).map_err(|_| {
            ClientError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Upload-ID ist zu lang",
            ))
        })?;
        let data = read_verified_range(&path, offset, &descriptor)?;
        content_bytes = content_bytes.saturating_add(data.len() as u64);
        payload.extend_from_slice(&upload_length.to_be_bytes());
        payload.extend_from_slice(upload_id.as_bytes());
        payload.extend_from_slice(descriptor.hash.as_bytes());
        payload.extend_from_slice(&descriptor.size.to_be_bytes());
        payload.extend_from_slice(&data);
    }
    Ok(EncodedBlockBatch {
        payload,
        content_bytes,
    })
}

fn build_http() -> Result<reqwest::Client, ClientError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(5 * 60))
        .user_agent(concat!("Hedwig/", env!("CARGO_PKG_VERSION")));
    if let Some(path) =
        std::env::var_os("HEDWIG_CA_CERT").or_else(|| std::env::var_os("CLOUD_CA_CERT"))
    {
        let pem = std::fs::read(path)?;
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    Ok(builder.build()?)
}

fn api_error(status: StatusCode, bytes: &[u8]) -> ClientError {
    match serde_json::from_slice::<ErrorEnvelope>(bytes) {
        Ok(envelope) => ClientError::Api {
            status,
            code: envelope.error.code,
            message: envelope.error.message,
        },
        Err(_) => ClientError::Api {
            status,
            code: "http_error".into(),
            message: String::from_utf8_lossy(bytes).chars().take(300).collect(),
        },
    }
}

impl From<ChunkDescriptor> for BlockDescriptor {
    fn from(value: ChunkDescriptor) -> Self {
        Self {
            hash: value.hash,
            size: value.size,
        }
    }
}
