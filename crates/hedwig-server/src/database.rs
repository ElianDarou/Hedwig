use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::Context;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::{DateTime, Datelike, Duration, Utc};
use hedwig_core::IgnoreRules;
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha1::Sha1;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthContext {
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub device_name: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    View,
    Upload,
    Edit,
    Manage,
    Own,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResult {
    pub token: String,
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub personal_space_id: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub editor_sharing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub id: String,
    pub path: String,
    pub size: i64,
    pub current_version_id: Option<String>,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryRow {
    pub id: String,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBlock {
    pub hash: String,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct UploadRow {
    pub id: String,
    pub space_id: String,
    pub user_id: String,
    pub device_id: String,
    pub path: String,
    pub file_id: Option<String>,
    pub base_version_id: Option<String>,
    pub size: i64,
    pub blocks: Vec<StoredBlock>,
    pub idempotency_key: String,
    pub expires_at: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct NewUpload {
    pub path: String,
    pub file_id: Option<String>,
    pub base_version_id: Option<String>,
    pub size: i64,
    pub blocks: Vec<StoredBlock>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResult {
    pub file_id: String,
    pub version_id: String,
    pub path: String,
    pub conflict: bool,
    pub change_sequence: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeRow {
    pub sequence: i64,
    pub event_id: String,
    pub space_id: String,
    pub object_id: String,
    pub kind: String,
    pub path: String,
    pub version_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionRow {
    pub id: String,
    pub size: i64,
    pub created_by: String,
    pub device_name: String,
    pub created_at: String,
    pub pinned_name: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MutationResult {
    pub file_id: String,
    pub path: String,
    pub change_sequence: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub is_admin: bool,
    pub created_at: String,
    pub disabled: bool,
    pub quota_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminSpaceRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub owner: String,
    pub quota_bytes: Option<i64>,
    pub archive_protected: bool,
    pub current_bytes: i64,
    pub version_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareRow {
    pub id: String,
    pub space_id: String,
    pub target_path: String,
    pub file_id: Option<String>,
    pub mode: String,
    pub password_protected: bool,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub max_upload_size: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CreatedShare {
    pub share: ShareRow,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct ShareAccess {
    pub share: ShareRow,
    pub auth: AuthContext,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen_at: String,
    pub revoked_at: Option<String>,
    pub active_sessions: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageUsage {
    pub current_bytes: i64,
    pub version_bytes: i64,
    pub physical_bytes: i64,
    pub quota_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemStatus {
    pub users: i64,
    pub spaces: i64,
    pub current_files: i64,
    pub trashed_files: i64,
    pub current_bytes: i64,
    pub version_bytes: i64,
    pub physical_bytes: i64,
    pub open_uploads: i64,
    pub expired_uploads: i64,
    pub pending_webhooks: i64,
    pub changes: i64,
    pub schema_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupStatus {
    pub id: String,
    pub destination: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub entries: Option<i64>,
    pub bytes: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderRestoreAction {
    pub file_id: String,
    pub path: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderRestorePreview {
    pub space_id: String,
    pub folder_path: String,
    pub target_at: String,
    pub actions: Vec<FolderRestoreAction>,
    pub unavailable: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderRestoreResult {
    pub restored_files: usize,
    pub deleted_files: usize,
}

#[derive(Debug, Clone)]
struct HistoricalFileState {
    file_id: String,
    path: String,
    version_id: Option<String>,
    deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetentionResult {
    pub deleted_versions: usize,
    pub purged_files: usize,
    pub unused_blocks: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRow {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub member_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceMemberRow {
    pub user_id: String,
    pub username: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceGroupRoleRow {
    pub group_id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessChoice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceAccess {
    pub members: Vec<SpaceMemberRow>,
    pub group_roles: Vec<SpaceGroupRoleRow>,
    pub available_users: Vec<AccessChoice>,
    pub available_groups: Vec<AccessChoice>,
}

#[derive(Debug, Clone)]
pub struct ExportEntry {
    pub space_id: String,
    pub space_name: String,
    pub file_id: String,
    pub path: String,
    pub version_id: String,
    pub pinned_name: Option<String>,
    pub created_at: String,
    pub current: bool,
    pub blocks: Vec<StoredBlock>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TwoFactorEnrollment {
    pub secret: String,
    pub otpauth_uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookRow {
    pub id: String,
    pub space_id: String,
    pub url: String,
    pub created_at: String,
    pub disabled_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreatedWebhook {
    pub webhook: WebhookRow,
    pub signing_secret: String,
}

#[derive(Debug, Clone)]
pub struct WebhookDelivery {
    pub id: String,
    pub event_id: String,
    pub url: String,
    pub signing_secret: String,
    pub payload: String,
    pub attempts: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("nicht angemeldet")]
    Unauthorized,
    #[error("keine Berechtigung")]
    Forbidden,
    #[error("nicht gefunden")]
    NotFound,
    #[error("der Server wurde bereits eingerichtet")]
    AlreadySetup,
    #[error("Benutzername oder Passwort ist falsch")]
    InvalidLogin,
    #[error("Konflikt: {0}")]
    Conflict(String),
    #[error("ungültige Anfrage: {0}")]
    Invalid(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut connection = Connection::open(path.as_ref())
            .with_context(|| format!("cannot open database {}", path.as_ref().display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
        )?;
        connection.execute_batch(SCHEMA)?;
        migrate_schema(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DbError> {
        self.connection
            .lock()
            .map_err(|_| DbError::Internal(anyhow::anyhow!("database lock poisoned")))
    }

    pub fn setup_required(&self) -> Result<bool, DbError> {
        let connection = self.lock()?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        Ok(count == 0)
    }

    pub fn schema_version(&self) -> Result<i64, DbError> {
        let connection = self.lock()?;
        Ok(connection.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn upload_policy(&self, auth: &AuthContext) -> Result<String, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        Ok(connection
            .query_row(
                "SELECT value FROM server_settings WHERE key='forbidden_upload_patterns'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_upload_policy(&self, auth: &AuthContext, patterns: &str) -> Result<(), DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        if patterns.len() > 32 * 1024 {
            return Err(DbError::Invalid(
                "Upload-Sperrliste darf höchstens 32 KiB groß sein".into(),
            ));
        }
        IgnoreRules::parse(patterns, "")
            .map_err(|error| DbError::Invalid(format!("Ungültige Upload-Regel: {error}")))?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO server_settings (key,value) VALUES ('forbidden_upload_patterns',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [patterns],
        )?;
        Ok(())
    }

    pub fn ensure_upload_allowed(&self, path: &str) -> Result<(), DbError> {
        let connection = self.lock()?;
        let patterns: String = connection
            .query_row(
                "SELECT value FROM server_settings WHERE key='forbidden_upload_patterns'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default();
        drop(connection);
        if patterns.trim().is_empty() {
            return Ok(());
        }
        let rules = IgnoreRules::parse(&patterns, "")
            .map_err(|error| DbError::Internal(anyhow::anyhow!(error)))?;
        let decision = rules.evaluate(Path::new(path), false);
        if decision.ignored {
            return Err(DbError::Invalid(format!(
                "Upload von {path} ist durch die serverweite Regel {} gesperrt",
                decision.rule.unwrap_or_else(|| "(unbekannt)".into())
            )));
        }
        Ok(())
    }

    pub fn snapshot_to(&self, destination: &Path) -> Result<(), DbError> {
        let connection = self.lock()?;
        connection.execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        Ok(())
    }

    pub fn integrity_check(&self) -> Result<(), DbError> {
        let connection = self.lock()?;
        let result: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if result != "ok" {
            return Err(DbError::Internal(anyhow::anyhow!(
                "SQLite integrity_check: {result}"
            )));
        }
        Ok(())
    }

    pub fn system_status(&self, auth: &AuthContext) -> Result<SystemStatus, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        let now = Utc::now().to_rfc3339();
        Ok(SystemStatus {
            users: connection.query_row(
                "SELECT COUNT(*) FROM users WHERE disabled_at IS NULL",
                [],
                |row| row.get(0),
            )?,
            spaces: connection.query_row(
                "SELECT COUNT(*) FROM spaces WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?,
            current_files: connection.query_row(
                "SELECT COUNT(*) FROM files f JOIN spaces s ON s.id=f.space_id
                 WHERE f.deleted_at IS NULL AND s.deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?,
            trashed_files: connection.query_row(
                "SELECT COUNT(*) FROM files f JOIN spaces s ON s.id=f.space_id
                 WHERE f.deleted_at IS NOT NULL AND s.deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?,
            current_bytes: connection.query_row(
                "SELECT COALESCE(SUM(f.size),0) FROM files f JOIN spaces s ON s.id=f.space_id
                 WHERE f.deleted_at IS NULL AND s.deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?,
            version_bytes: connection.query_row(
                "SELECT COALESCE(SUM(size),0) FROM versions",
                [],
                |row| row.get(0),
            )?,
            physical_bytes: connection.query_row(
                "SELECT COALESCE(SUM(size),0) FROM blocks WHERE reference_count>0",
                [],
                |row| row.get(0),
            )?,
            open_uploads: connection.query_row(
                "SELECT COUNT(*) FROM uploads WHERE status='open'",
                [],
                |row| row.get(0),
            )?,
            expired_uploads: connection.query_row(
                "SELECT COUNT(*) FROM uploads WHERE status='open' AND expires_at<=?1",
                [&now],
                |row| row.get(0),
            )?,
            pending_webhooks: connection.query_row(
                "SELECT COUNT(*) FROM webhook_deliveries WHERE status='pending'",
                [],
                |row| row.get(0),
            )?,
            changes: connection.query_row("SELECT COUNT(*) FROM changes", [], |row| row.get(0))?,
            schema_version: connection.query_row(
                "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )?,
        })
    }

    pub fn start_backup_run(&self, destination: &Path) -> Result<String, DbError> {
        let connection = self.lock()?;
        let id = Uuid::now_v7().to_string();
        connection.execute(
            "INSERT INTO backup_runs (id,destination,started_at,status) VALUES (?1,?2,?3,'running')",
            params![id, destination.to_string_lossy(), Utc::now().to_rfc3339()],
        )?;
        Ok(id)
    }

    pub fn finish_backup_run(
        &self,
        id: &str,
        entries: Option<i64>,
        bytes: Option<i64>,
        error: Option<&str>,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE backup_runs SET completed_at=?1,status=?2,entries=?3,bytes=?4,error=?5 WHERE id=?6",
            params![Utc::now().to_rfc3339(), if error.is_some() { "failed" } else { "completed" }, entries, bytes,
                error.map(|value| value.chars().take(2000).collect::<String>()), id],
        )?;
        Ok(())
    }

    pub fn latest_backup_status(
        &self,
        auth: &AuthContext,
    ) -> Result<Option<BackupStatus>, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        Ok(connection
            .query_row(
                "SELECT id,destination,started_at,completed_at,status,entries,bytes,error FROM backup_runs ORDER BY started_at DESC LIMIT 1",
                [],
                |row| Ok(BackupStatus { id: row.get(0)?, destination: row.get(1)?, started_at: row.get(2)?,
                    completed_at: row.get(3)?, status: row.get(4)?, entries: row.get(5)?, bytes: row.get(6)?, error: row.get(7)? }),
            )
            .optional()?)
    }

    pub fn apply_default_retention(&self, now: DateTime<Utc>) -> Result<RetentionResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE uploads SET status='aborted' WHERE status='open' AND expires_at<=?1",
            [now.to_rfc3339()],
        )?;
        let trash_cutoff = (now - Duration::days(30)).to_rfc3339();
        let purged_files: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT f.id FROM files f JOIN spaces s ON s.id=f.space_id
                 WHERE ((f.deleted_at IS NOT NULL AND f.deleted_at<?1)
                        OR (s.deleted_at IS NOT NULL AND s.deleted_at<?1))
                   AND COALESCE(s.archive_protected,0)=0",
            )?;
            statement
                .query_map([&trash_cutoff], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let purge_set: HashSet<&str> = purged_files.iter().map(String::as_str).collect();
        let versions: Vec<(String, String, String, Option<String>, bool, bool)> = {
            let mut statement = transaction.prepare(
                "SELECT v.id,v.file_id,v.created_at,v.pinned_name,v.id=f.current_version_id,s.archive_protected
                 FROM versions v JOIN files f ON f.id=v.file_id JOIN spaces s ON s.id=f.space_id
                 ORDER BY v.file_id,v.created_at DESC",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut seen_buckets = HashSet::new();
        let mut delete_versions = Vec::new();
        for (version_id, file_id, created_at, pinned_name, current, archive_protected) in versions {
            if purge_set.contains(file_id.as_str()) {
                delete_versions.push(version_id);
                continue;
            }
            if current || pinned_name.is_some() || archive_protected {
                continue;
            }
            let created = DateTime::parse_from_rfc3339(&created_at)
                .map_err(|error| DbError::Internal(error.into()))?
                .with_timezone(&Utc);
            let age = now.signed_duration_since(created);
            if age < Duration::hours(24) {
                continue;
            }
            let bucket = if age < Duration::days(7) {
                format!("{file_id}:hour:{}", created.format("%Y-%m-%d-%H"))
            } else if age < Duration::days(90) {
                format!("{file_id}:day:{}", created.format("%Y-%m-%d"))
            } else if age < Duration::days(730) {
                let week = created.iso_week();
                format!("{file_id}:week:{}-{}", week.year(), week.week())
            } else {
                delete_versions.push(version_id);
                continue;
            };
            if !seen_buckets.insert(bucket) {
                delete_versions.push(version_id);
            }
        }
        for version_id in &delete_versions {
            let blocks: Vec<(String, String)> = {
                let mut statement = transaction.prepare(
                    "SELECT f.space_id,vb.block_hash FROM version_blocks vb JOIN versions v ON v.id=vb.version_id
                     JOIN files f ON f.id=v.file_id WHERE vb.version_id=?1",
                )?;
                statement
                    .query_map([version_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            transaction.execute(
                "DELETE FROM version_blocks WHERE version_id=?1",
                [version_id],
            )?;
            transaction.execute("DELETE FROM versions WHERE id=?1", [version_id])?;
            for (space_id, hash) in blocks {
                transaction.execute(
                    "UPDATE blocks SET reference_count=MAX(reference_count-1,0) WHERE space_id=?1 AND hash=?2",
                    params![space_id, hash],
                )?;
            }
        }
        for file_id in &purged_files {
            transaction.execute("DELETE FROM shares WHERE file_id=?1", [file_id])?;
            transaction.execute("DELETE FROM files WHERE id=?1", [file_id])?;
        }
        transaction.execute(
            "DELETE FROM directories WHERE space_id IN
             (SELECT id FROM spaces WHERE deleted_at IS NOT NULL AND deleted_at<?1 AND COALESCE(archive_protected,0)=0)",
            [&trash_cutoff],
        )?;
        let active_upload_blocks: HashSet<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT space_id,blocks_json FROM uploads WHERE status='open' AND expires_at>?1",
            )?;
            let rows = statement
                .query_map([now.to_rfc3339()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut protected = HashSet::new();
            for (space_id, json) in rows {
                let blocks: Vec<StoredBlock> = serde_json::from_str(&json)?;
                protected.extend(
                    blocks
                        .into_iter()
                        .map(|block| (space_id.clone(), block.hash)),
                );
            }
            protected
        };
        let unused_blocks: Vec<(String, String)> = {
            let mut statement =
                transaction.prepare("SELECT space_id,hash FROM blocks WHERE reference_count<=0")?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|block| !active_upload_blocks.contains(block))
                .collect()
        };
        for (space_id, hash) in &unused_blocks {
            transaction.execute(
                "DELETE FROM blocks WHERE space_id=?1 AND hash=?2",
                params![space_id, hash],
            )?;
        }
        transaction.commit()?;
        Ok(RetentionResult {
            deleted_versions: delete_versions.len(),
            purged_files: purged_files.len(),
            unused_blocks,
        })
    }

    pub fn protected_blocks(
        &self,
        now: DateTime<Utc>,
    ) -> Result<HashSet<(String, String)>, DbError> {
        let connection = self.lock()?;
        let mut protected: HashSet<(String, String)> = {
            let mut statement =
                connection.prepare("SELECT space_id,hash FROM blocks WHERE reference_count>0")?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<HashSet<_>, _>>()?
        };
        let uploads: Vec<(String, String)> = {
            let mut statement = connection.prepare(
                "SELECT space_id,blocks_json FROM uploads WHERE status='open' AND expires_at>?1",
            )?;
            statement
                .query_map([now.to_rfc3339()], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (space_id, json) in uploads {
            let blocks: Vec<StoredBlock> = serde_json::from_str(&json)?;
            protected.extend(
                blocks
                    .into_iter()
                    .map(|block| (space_id.clone(), block.hash)),
            );
        }
        Ok(protected)
    }

    pub fn export_entries(&self, include_pinned: bool) -> Result<Vec<ExportEntry>, DbError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT s.id,s.name,f.id,f.path,v.id,v.pinned_name,v.created_at,v.id=f.current_version_id
             FROM versions v JOIN files f ON f.id=v.file_id JOIN spaces s ON s.id=f.space_id
             WHERE f.deleted_at IS NULL AND (v.id=f.current_version_id OR (?1 AND v.pinned_name IS NOT NULL))
             ORDER BY s.name,f.path,v.created_at",
        )?;
        let rows = statement
            .query_map([include_pinned], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, bool>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut entries = Vec::with_capacity(rows.len());
        for (space_id, space_name, file_id, path, version_id, pinned_name, created_at, current) in
            rows
        {
            let mut block_statement = connection.prepare(
                "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
            )?;
            let blocks = block_statement
                .query_map([&version_id], |row| {
                    Ok(StoredBlock {
                        hash: row.get(0)?,
                        size: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            entries.push(ExportEntry {
                space_id,
                space_name,
                file_id,
                path,
                version_id,
                pinned_name,
                created_at,
                current,
                blocks,
            });
        }
        Ok(entries)
    }

    pub fn setup_admin(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
    ) -> Result<SessionResult, DbError> {
        validate_credentials(username, password)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        if count != 0 {
            return Err(DbError::AlreadySetup);
        }
        let user_id = Uuid::now_v7().to_string();
        let space_id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        let password_hash = hash_password(password)?;
        transaction.execute(
            "INSERT INTO users (id, username, password_hash, is_admin, created_at) VALUES (?1, ?2, ?3, 1, ?4)",
            params![user_id, username, password_hash, now],
        )?;
        transaction.execute(
            "INSERT INTO spaces (id, name, kind, owner_user_id, created_at) VALUES (?1, ?2, 'personal', ?3, ?4)",
            params![space_id, format!("Dateien von {username}"), user_id, now],
        )?;
        transaction.execute(
            "INSERT INTO memberships (space_id, user_id, role) VALUES (?1, ?2, 'owner')",
            params![space_id, user_id],
        )?;
        let result = create_session(
            &transaction,
            &user_id,
            username,
            true,
            device_name,
            &space_id,
        )?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn login(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
    ) -> Result<SessionResult, DbError> {
        self.login_with_otp(username, password, device_name, None)
    }

    pub fn login_with_otp(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
        otp: Option<&str>,
    ) -> Result<SessionResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let user = transaction
            .query_row(
                "SELECT id,password_hash,is_admin,totp_cipher,totp_salt,totp_nonce
                 FROM users WHERE username = ?1 COLLATE NOCASE AND disabled_at IS NULL",
                [username],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((user_id, password_hash, is_admin, cipher, salt, nonce)) = user else {
            return Err(DbError::InvalidLogin);
        };
        let parsed = PasswordHash::new(&password_hash).map_err(|_| DbError::InvalidLogin)?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            return Err(DbError::InvalidLogin);
        }
        match (cipher, salt, nonce) {
            (Some(cipher), Some(salt), Some(nonce)) => {
                let secret = decrypt_secret(password, &user_id, &cipher, &salt, &nonce)
                    .map_err(|_| DbError::InvalidLogin)?;
                if !otp.is_some_and(|code| verify_totp(&secret, code, Utc::now().timestamp())) {
                    return Err(DbError::InvalidLogin);
                }
            }
            (None, None, None) => {}
            _ => {
                return Err(DbError::Internal(anyhow::anyhow!(
                    "unvollständige 2FA-Daten"
                )));
            }
        }
        let personal_space_id: String = transaction.query_row(
            "SELECT id FROM spaces WHERE owner_user_id = ?1 AND kind = 'personal' LIMIT 1",
            [&user_id],
            |row| row.get(0),
        )?;
        let result = create_session(
            &transaction,
            &user_id,
            username,
            is_admin,
            device_name,
            &personal_space_id,
        )?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn authenticate(&self, raw_token: &str) -> Result<AuthContext, DbError> {
        let token_hash = hash_token(raw_token);
        let connection = self.lock()?;
        let auth = connection
            .query_row(
                "SELECT u.id, u.username, d.id, d.name, u.is_admin
                 FROM sessions s JOIN devices d ON d.id = s.device_id JOIN users u ON u.id = d.user_id
                 WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2
                   AND d.revoked_at IS NULL AND u.disabled_at IS NULL",
                params![token_hash, Utc::now().to_rfc3339()],
                |row| Ok(AuthContext {
                    user_id: row.get(0)?, username: row.get(1)?, device_id: row.get(2)?,
                    device_name: row.get(3)?, is_admin: row.get(4)?,
                }),
            )
            .optional()?
            .ok_or(DbError::Unauthorized)?;
        if let Err(error) = connection.execute(
            "UPDATE devices SET last_seen_at=?1 WHERE id=?2",
            params![Utc::now().to_rfc3339(), auth.device_id],
        ) {
            // Authentication and read access must remain available when the
            // metadata volume is full. Last-seen tracking is operational
            // telemetry and must not turn an otherwise valid read into data
            // unavailability.
            tracing::warn!(%error, device_id = %auth.device_id, "device activity timestamp could not be updated");
        }
        Ok(auth)
    }

    pub fn logout(&self, raw_token: &str) -> Result<(), DbError> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE sessions SET revoked_at=?1 WHERE token_hash=?2 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), hash_token(raw_token)],
        )?;
        Ok(())
    }

    pub fn list_devices(&self, auth: &AuthContext) -> Result<Vec<DeviceRow>, DbError> {
        let connection = self.lock()?;
        let now = Utc::now().to_rfc3339();
        let mut statement = connection.prepare(
            "SELECT d.id,d.name,d.created_at,d.last_seen_at,d.revoked_at,
                    COUNT(CASE WHEN s.revoked_at IS NULL AND s.expires_at>?2 THEN 1 END)
             FROM devices d LEFT JOIN sessions s ON s.device_id=d.id
             WHERE d.user_id=?1 GROUP BY d.id ORDER BY d.last_seen_at DESC",
        )?;
        Ok(statement
            .query_map(params![auth.user_id, now], |row| {
                Ok(DeviceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    last_seen_at: row.get(3)?,
                    revoked_at: row.get(4)?,
                    active_sessions: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn revoke_device(&self, auth: &AuthContext, device_id: &str) -> Result<(), DbError> {
        let connection = self.lock()?;
        let changed = connection.execute(
            "UPDATE devices SET revoked_at=?1 WHERE id=?2 AND user_id=?3 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), device_id, auth.user_id],
        )?;
        if changed == 0 {
            let exists = connection
                .query_row(
                    "SELECT 1 FROM devices WHERE id=?1 AND user_id=?2",
                    params![device_id, auth.user_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Err(DbError::NotFound);
            }
        }
        connection.execute(
            "UPDATE sessions SET revoked_at=COALESCE(revoked_at,?1) WHERE device_id=?2",
            params![Utc::now().to_rfc3339(), device_id],
        )?;
        Ok(())
    }

    pub fn change_password(
        &self,
        auth: &AuthContext,
        current_password: &str,
        new_password: &str,
        current_token: &str,
    ) -> Result<(), DbError> {
        validate_credentials(&auth.username, new_password)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let existing: String = transaction.query_row(
            "SELECT password_hash FROM users WHERE id=?1 AND disabled_at IS NULL",
            [&auth.user_id],
            |row| row.get(0),
        )?;
        let parsed = PasswordHash::new(&existing).map_err(|_| DbError::InvalidLogin)?;
        Argon2::default()
            .verify_password(current_password.as_bytes(), &parsed)
            .map_err(|_| DbError::InvalidLogin)?;
        transaction.execute(
            "UPDATE users SET password_hash=?1 WHERE id=?2",
            params![hash_password(new_password)?, auth.user_id],
        )?;
        rekey_two_factor(&transaction, &auth.user_id, current_password, new_password)?;
        transaction.execute(
            "UPDATE sessions SET revoked_at=?1 WHERE device_id IN (SELECT id FROM devices WHERE user_id=?2)
             AND token_hash<>?3 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), auth.user_id, hash_token(current_token)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn two_factor_status(&self, auth: &AuthContext) -> Result<bool, DbError> {
        let connection = self.lock()?;
        Ok(connection.query_row(
            "SELECT totp_cipher IS NOT NULL FROM users WHERE id=?1",
            [&auth.user_id],
            |row| row.get(0),
        )?)
    }

    pub fn begin_two_factor(
        &self,
        auth: &AuthContext,
        password: &str,
    ) -> Result<TwoFactorEnrollment, DbError> {
        let connection = self.lock()?;
        verify_account_password(&connection, &auth.user_id, password)?;
        let secret = rand::random::<[u8; 20]>();
        let (cipher, salt, nonce) = encrypt_secret(password, &auth.user_id, &secret)?;
        connection.execute(
            "UPDATE users SET pending_totp_cipher=?1,pending_totp_salt=?2,pending_totp_nonce=?3 WHERE id=?4",
            params![cipher,salt,nonce,auth.user_id],
        )?;
        let encoded = base32_encode(&secret);
        let label: String =
            url::form_urlencoded::byte_serialize(format!("Hedwig:{}", auth.username).as_bytes())
                .collect();
        Ok(TwoFactorEnrollment {
            secret: encoded.clone(),
            otpauth_uri: format!(
                "otpauth://totp/{label}?secret={encoded}&issuer=Cloud%20Sync&algorithm=SHA1&digits=6&period=30"
            ),
        })
    }

    pub fn confirm_two_factor(
        &self,
        auth: &AuthContext,
        password: &str,
        code: &str,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        verify_account_password(&connection, &auth.user_id, password)?;
        let pending = load_encrypted_secret(&connection, &auth.user_id, true)?
            .ok_or_else(|| DbError::Conflict("Keine ausstehende 2FA-Einrichtung".into()))?;
        let secret = decrypt_secret(password, &auth.user_id, &pending.0, &pending.1, &pending.2)
            .map_err(|_| DbError::InvalidLogin)?;
        if !verify_totp(&secret, code, Utc::now().timestamp()) {
            return Err(DbError::InvalidLogin);
        }
        connection.execute(
            "UPDATE users SET totp_cipher=pending_totp_cipher,totp_salt=pending_totp_salt,totp_nonce=pending_totp_nonce,
             pending_totp_cipher=NULL,pending_totp_salt=NULL,pending_totp_nonce=NULL WHERE id=?1",
            [&auth.user_id],
        )?;
        Ok(())
    }

    pub fn disable_two_factor(
        &self,
        auth: &AuthContext,
        password: &str,
        code: &str,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        verify_account_password(&connection, &auth.user_id, password)?;
        let active = load_encrypted_secret(&connection, &auth.user_id, false)?
            .ok_or_else(|| DbError::Conflict("2FA ist nicht aktiviert".into()))?;
        let secret = decrypt_secret(password, &auth.user_id, &active.0, &active.1, &active.2)
            .map_err(|_| DbError::InvalidLogin)?;
        if !verify_totp(&secret, code, Utc::now().timestamp()) {
            return Err(DbError::InvalidLogin);
        }
        connection.execute(
            "UPDATE users SET totp_cipher=NULL,totp_salt=NULL,totp_nonce=NULL,
             pending_totp_cipher=NULL,pending_totp_salt=NULL,pending_totp_nonce=NULL WHERE id=?1",
            [&auth.user_id],
        )?;
        Ok(())
    }

    pub fn regenerate_recovery_codes(&self, auth: &AuthContext) -> Result<Vec<String>, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM recovery_codes WHERE user_id=?1",
            [&auth.user_id],
        )?;
        let now = Utc::now().to_rfc3339();
        let codes: Vec<String> = (0..10)
            .map(|_| {
                let raw = hex::encode(rand::random::<[u8; 8]>());
                format!(
                    "{}-{}-{}-{}",
                    &raw[0..4],
                    &raw[4..8],
                    &raw[8..12],
                    &raw[12..16]
                )
            })
            .collect();
        for code in &codes {
            transaction.execute(
                "INSERT INTO recovery_codes (id,user_id,code_hash,created_at) VALUES (?1,?2,?3,?4)",
                params![
                    Uuid::now_v7().to_string(),
                    auth.user_id,
                    hash_token(code),
                    now
                ],
            )?;
        }
        transaction.commit()?;
        Ok(codes)
    }

    pub fn recover(
        &self,
        username: &str,
        recovery_code: &str,
        device_name: &str,
    ) -> Result<SessionResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let account = transaction
            .query_row(
                "SELECT u.id,u.username,u.is_admin,r.id FROM users u JOIN recovery_codes r ON r.user_id=u.id
                 WHERE u.username=?1 COLLATE NOCASE AND r.code_hash=?2 AND r.used_at IS NULL AND u.disabled_at IS NULL",
                params![username, hash_token(recovery_code)],
                |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, bool>(2)?,row.get::<_, String>(3)?)),
            )
            .optional()?
            .ok_or(DbError::InvalidLogin)?;
        transaction.execute(
            "UPDATE recovery_codes SET used_at=?1 WHERE id=?2",
            params![Utc::now().to_rfc3339(), account.3],
        )?;
        let personal_space_id: String = transaction.query_row(
            "SELECT id FROM spaces WHERE owner_user_id=?1 AND kind='personal' LIMIT 1",
            [&account.0],
            |row| row.get(0),
        )?;
        let result = create_session(
            &transaction,
            &account.0,
            &account.1,
            account.2,
            device_name,
            &personal_space_id,
        )?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn create_share(
        &self,
        auth: &AuthContext,
        space_id: &str,
        target_path: &str,
        mode: &str,
        password: Option<&str>,
        expires_at: Option<&str>,
        max_upload_size: Option<i64>,
    ) -> Result<CreatedShare, DbError> {
        if !matches!(mode, "read" | "upload") {
            return Err(DbError::Invalid(
                "Freigabemodus muss read oder upload sein".into(),
            ));
        }
        if password.is_some_and(|value| value.chars().count() < 8) {
            return Err(DbError::Invalid(
                "Freigabepasswort muss mindestens 8 Zeichen lang sein".into(),
            ));
        }
        let expires_at = expires_at
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|parsed| parsed.with_timezone(&Utc).to_rfc3339())
                    .map_err(|_| DbError::Invalid("Ablaufdatum ist ungültig".into()))
            })
            .transpose()?;
        let now_text = Utc::now().to_rfc3339();
        if expires_at
            .as_deref()
            .is_some_and(|value| value <= now_text.as_str())
        {
            return Err(DbError::Invalid(
                "Ablaufdatum liegt nicht in der Zukunft".into(),
            ));
        }
        if max_upload_size.is_some_and(|size| size <= 0) {
            return Err(DbError::Invalid("Upload-Limit muss positiv sein".into()));
        }

        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        require_share_permission(&transaction, space_id, &auth.user_id)?;
        let file_id = transaction
            .query_row(
                "SELECT id FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, target_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if mode == "read" && file_id.is_none() {
            let prefix = format!("{}/%", escape_like(target_path));
            let has_child_file = transaction
                .query_row(
                    "SELECT 1 FROM files WHERE space_id=?1 AND path LIKE ?2 ESCAPE '\\' AND deleted_at IS NULL LIMIT 1",
                    params![space_id, prefix],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            let has_directory = transaction
                .query_row(
                    "SELECT 1 FROM directories WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                    params![space_id, target_path],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !has_child_file && !has_directory {
                return Err(DbError::NotFound);
            }
        }

        let id = Uuid::now_v7().to_string();
        let token = format!("cl_{}", hex::encode(rand::random::<[u8; 32]>()));
        let password_hash = password.map(hash_password).transpose()?;
        let device_id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO devices (id,user_id,name,created_at,last_seen_at) VALUES (?1,?2,?3,?4,?4)",
            params![device_id, auth.user_id, format!("Freigabelink {id}"), now],
        )?;
        transaction.execute(
            "INSERT INTO shares (id,token_hash,space_id,target_path,file_id,mode,password_hash,expires_at,max_upload_size,created_by_user_id,guest_device_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![id,hash_token(&token),space_id,target_path,file_id,mode,password_hash,expires_at,max_upload_size,auth.user_id,device_id,now],
        )?;
        insert_change(
            &transaction,
            space_id,
            &id,
            "share_created",
            target_path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(CreatedShare {
            share: ShareRow {
                id,
                space_id: space_id.into(),
                target_path: target_path.into(),
                file_id,
                mode: mode.into(),
                password_protected: password_hash.is_some(),
                expires_at,
                revoked_at: None,
                max_upload_size,
                created_at: now,
            },
            token,
        })
    }

    pub fn create_webhook(
        &self,
        auth: &AuthContext,
        space_id: &str,
        target_url: &str,
    ) -> Result<CreatedWebhook, DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let parsed = url::Url::parse(target_url)
            .map_err(|_| DbError::Invalid("Webhook-URL ist ungültig".into()))?;
        let local_http = parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
        if parsed.scheme() != "https" && !local_http {
            return Err(DbError::Invalid("Webhook-URL muss HTTPS verwenden".into()));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DbError::Invalid(
                "Webhook-URL darf keine Zugangsdaten enthalten".into(),
            ));
        }
        let id = Uuid::now_v7().to_string();
        let signing_secret = format!("whsec_{}", hex::encode(rand::random::<[u8; 32]>()));
        let now = Utc::now().to_rfc3339();
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO webhooks (id,space_id,url,signing_secret,created_by_user_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                id,
                space_id,
                parsed.as_str(),
                signing_secret,
                auth.user_id,
                now
            ],
        )?;
        Ok(CreatedWebhook {
            webhook: WebhookRow {
                id,
                space_id: space_id.into(),
                url: parsed.to_string(),
                created_at: now,
                disabled_at: None,
            },
            signing_secret,
        })
    }

    pub fn list_webhooks(
        &self,
        auth: &AuthContext,
        space_id: &str,
    ) -> Result<Vec<WebhookRow>, DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id,space_id,url,created_at,disabled_at FROM webhooks WHERE space_id=?1 ORDER BY created_at DESC",
        )?;
        Ok(statement
            .query_map([space_id], |row| {
                Ok(WebhookRow {
                    id: row.get(0)?,
                    space_id: row.get(1)?,
                    url: row.get(2)?,
                    created_at: row.get(3)?,
                    disabled_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn disable_webhook(
        &self,
        auth: &AuthContext,
        space_id: &str,
        webhook_id: &str,
    ) -> Result<(), DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let connection = self.lock()?;
        if connection.execute(
            "UPDATE webhooks SET disabled_at=?1 WHERE id=?2 AND space_id=?3 AND disabled_at IS NULL",
            params![Utc::now().to_rfc3339(),webhook_id,space_id],
        )? == 0 { return Err(DbError::NotFound); }
        Ok(())
    }

    pub fn due_webhook_deliveries(&self, limit: usize) -> Result<Vec<WebhookDelivery>, DbError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT wd.id,c.event_id,w.url,w.signing_secret,wd.attempts,c.kind,c.space_id,c.object_id,c.path,c.version_id,c.created_at
             FROM webhook_deliveries wd JOIN webhooks w ON w.id=wd.webhook_id
             JOIN changes c ON c.event_id=wd.event_id
             WHERE wd.status='pending' AND wd.next_attempt_at<=?1 AND w.disabled_at IS NULL
             ORDER BY wd.next_attempt_at LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![Utc::now().to_rfc3339(), limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|row| WebhookDelivery {
                id: row.0,
                event_id: row.1.clone(),
                url: row.2,
                signing_secret: row.3,
                attempts: row.4,
                payload: json!({
                    "event_id":row.1,"type":row.5,"space_id":row.6,"object_id":row.7,
                    "path":row.8,"version_id":row.9,"occurred_at":row.10,
                })
                .to_string(),
            })
            .collect())
    }

    pub fn complete_webhook_delivery(
        &self,
        delivery: &WebhookDelivery,
        success: bool,
        error: Option<&str>,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        if success {
            connection.execute(
                "UPDATE webhook_deliveries SET status='delivered',attempts=attempts+1,delivered_at=?1,last_error=NULL WHERE id=?2",
                params![Utc::now().to_rfc3339(),delivery.id],
            )?;
        } else {
            let attempts = delivery.attempts + 1;
            let delay = 2_i64.saturating_pow(attempts.min(16) as u32).min(86_400);
            connection.execute(
                "UPDATE webhook_deliveries SET attempts=?1,next_attempt_at=?2,last_error=?3 WHERE id=?4",
                params![attempts,(Utc::now()+Duration::seconds(delay)).to_rfc3339(),error.map(|value| value.chars().take(500).collect::<String>()),delivery.id],
            )?;
        }
        Ok(())
    }

    pub fn list_shares(&self, auth: &AuthContext) -> Result<Vec<ShareRow>, DbError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT s.id,s.space_id,s.target_path,s.file_id,s.mode,s.password_hash IS NOT NULL,s.expires_at,s.revoked_at,s.max_upload_size,s.created_at
             FROM shares s WHERE EXISTS (
               SELECT 1 FROM memberships m JOIN spaces permitted ON permitted.id=m.space_id
               WHERE m.space_id=s.space_id AND m.user_id=?1
                 AND (m.role IN ('owner','manager') OR (m.role='editor' AND permitted.editor_sharing=1))
               UNION ALL
               SELECT 1 FROM group_space_roles gsr JOIN group_members gm ON gm.group_id=gsr.group_id
               JOIN spaces permitted ON permitted.id=gsr.space_id
               WHERE gsr.space_id=s.space_id AND gm.user_id=?1
                 AND (gsr.role='manager' OR (gsr.role='editor' AND permitted.editor_sharing=1))
             ) ORDER BY s.created_at DESC",
        )?;
        Ok(statement
            .query_map([&auth.user_id], share_from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn revoke_share(&self, auth: &AuthContext, share_id: &str) -> Result<(), DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, target_path) = transaction
            .query_row(
                "SELECT space_id,target_path FROM shares WHERE id=?1",
                [share_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_share_permission(&transaction, &space_id, &auth.user_id)?;
        let changed = transaction.execute(
            "UPDATE shares SET revoked_at=?1 WHERE id=?2 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), share_id],
        )?;
        if changed > 0 {
            let now = Utc::now().to_rfc3339();
            insert_change(
                &transaction,
                &space_id,
                share_id,
                "share_revoked",
                &target_path,
                None,
                auth,
                &now,
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn authenticate_share(
        &self,
        raw_token: &str,
        password: Option<&str>,
    ) -> Result<ShareAccess, DbError> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT s.id,s.space_id,s.target_path,s.file_id,s.mode,s.password_hash,s.expires_at,s.revoked_at,s.max_upload_size,s.created_at,
                        u.id,u.username,u.is_admin,d.id,d.name
                 FROM shares s JOIN users u ON u.id=s.created_by_user_id JOIN devices d ON d.id=s.guest_device_id
                 WHERE s.token_hash=?1 AND u.disabled_at IS NULL AND d.revoked_at IS NULL",
                [hash_token(raw_token)],
                |row| Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<i64>>(8)?, row.get::<_, String>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?,
                    row.get::<_, bool>(12)?, row.get::<_, String>(13)?, row.get::<_, String>(14)?
                )),
            )
            .optional()?
            .ok_or(DbError::Unauthorized)?;
        let now_text = Utc::now().to_rfc3339();
        if row.7.is_some()
            || row
                .6
                .as_deref()
                .is_some_and(|expiry| expiry <= now_text.as_str())
        {
            return Err(DbError::Unauthorized);
        }
        if let Some(expected) = &row.5 {
            let supplied = password.ok_or(DbError::Unauthorized)?;
            let parsed = PasswordHash::new(expected).map_err(|_| DbError::Unauthorized)?;
            Argon2::default()
                .verify_password(supplied.as_bytes(), &parsed)
                .map_err(|_| DbError::Unauthorized)?;
        }
        Ok(ShareAccess {
            share: ShareRow {
                id: row.0,
                space_id: row.1,
                target_path: row.2,
                file_id: row.3,
                mode: row.4,
                password_protected: row.5.is_some(),
                expires_at: row.6,
                revoked_at: row.7,
                max_upload_size: row.8,
                created_at: row.9,
            },
            auth: AuthContext {
                user_id: row.10,
                username: row.11,
                is_admin: row.12,
                device_id: row.13,
                device_name: row.14,
            },
        })
    }

    pub fn share_files(&self, access: &ShareAccess) -> Result<Vec<FileRow>, DbError> {
        if access.share.mode != "read" {
            return Ok(Vec::new());
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id,path,size,current_version_id,updated_at,deleted_at FROM files
             WHERE space_id=?1 AND deleted_at IS NULL AND (id=?2 OR path LIKE ?3 ESCAPE '\\') ORDER BY path",
        )?;
        let prefix = format!("{}/%", escape_like(&access.share.target_path));
        Ok(statement
            .query_map(
                params![access.share.space_id, access.share.file_id, prefix],
                |row| {
                    Ok(FileRow {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        size: row.get(2)?,
                        current_version_id: row.get(3)?,
                        updated_at: row.get(4)?,
                        deleted_at: row.get(5)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn share_file_blocks(
        &self,
        access: &ShareAccess,
        file_id: &str,
    ) -> Result<(String, String, Vec<StoredBlock>), DbError> {
        if access.share.mode != "read" {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        let (space_id, path, version_id): (String, String, String) = connection
            .query_row(
                "SELECT space_id,path,current_version_id FROM files WHERE id=?1 AND deleted_at IS NULL",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        let inside_folder = path.starts_with(&format!("{}/", access.share.target_path));
        if space_id != access.share.space_id
            || (access.share.file_id.as_deref() != Some(file_id) && !inside_folder)
        {
            return Err(DbError::Forbidden);
        }
        let mut statement = connection.prepare(
            "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
        )?;
        let blocks = statement
            .query_map([version_id], |row| {
                Ok(StoredBlock {
                    hash: row.get(0)?,
                    size: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((space_id, path, blocks))
    }

    pub fn list_spaces(&self, user_id: &str) -> Result<Vec<SpaceRow>, DbError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT s.id,s.name,s.kind,m.role,s.editor_sharing FROM spaces s JOIN memberships m ON m.space_id=s.id
             WHERE m.user_id=?1 AND s.deleted_at IS NULL
             UNION ALL
             SELECT s.id,s.name,s.kind,gs.role,s.editor_sharing FROM spaces s JOIN group_space_roles gs ON gs.space_id=s.id
             JOIN group_members gm ON gm.group_id=gs.group_id
             WHERE gm.user_id=?1 AND s.deleted_at IS NULL",
        )?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok(SpaceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    role: row.get(3)?,
                    editor_sharing: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut combined: HashMap<String, SpaceRow> = HashMap::new();
        for row in rows {
            combined
                .entry(row.id.clone())
                .and_modify(|existing| {
                    if role_rank(&row.role) > role_rank(&existing.role) {
                        existing.role = row.role.clone();
                    }
                })
                .or_insert(row);
        }
        let mut spaces: Vec<_> = combined.into_values().collect();
        spaces.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(spaces)
    }

    pub fn create_space(&self, user_id: &str, name: &str) -> Result<SpaceRow, DbError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(DbError::Invalid(
                "Space-Name muss 1 bis 120 Zeichen lang sein".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO spaces (id, name, kind, owner_user_id, created_at) VALUES (?1, ?2, 'shared', ?3, ?4)",
            params![id, name, user_id, Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "INSERT INTO memberships (space_id,user_id,role) VALUES (?1,?2,'owner')",
            params![id, user_id],
        )?;
        transaction.commit()?;
        Ok(SpaceRow {
            id,
            name: name.into(),
            kind: "shared".into(),
            role: "owner".into(),
            editor_sharing: false,
        })
    }

    pub fn delete_space(
        &self,
        auth: &AuthContext,
        space_id: &str,
        confirmation_name: &str,
    ) -> Result<(), DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        require_permission_connection(&transaction, space_id, &auth.user_id, Permission::Own)?;
        let (name, kind, archive_protected): (String, String, bool) = transaction
            .query_row(
                "SELECT name,kind,archive_protected FROM spaces WHERE id=?1 AND deleted_at IS NULL",
                [space_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        if kind != "shared" {
            return Err(DbError::Conflict(
                "Der persönliche Speicherbereich kann nicht gelöscht werden".into(),
            ));
        }
        if archive_protected {
            return Err(DbError::Conflict(
                "Der Archivschutz muss vor dem Löschen durch einen Administrator aufgehoben werden"
                    .into(),
            ));
        }
        if confirmation_name != name {
            return Err(DbError::Invalid(
                "Der Bestätigungsname stimmt nicht mit dem Space-Namen überein".into(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE spaces SET deleted_at=?1 WHERE id=?2",
            params![now, space_id],
        )?;
        // The metadata remains available for the same 30-day safety window as
        // ordinary trash. Active counters exclude the deleted space, while
        // maintenance reclaims its versions and blocks after that window.
        transaction.execute(
            "UPDATE uploads SET status='aborted' WHERE space_id=?1 AND status='open'",
            [space_id],
        )?;
        transaction.execute(
            "UPDATE shares SET revoked_at=COALESCE(revoked_at,?1) WHERE space_id=?2",
            params![now, space_id],
        )?;
        transaction.execute(
            "UPDATE webhooks SET disabled_at=COALESCE(disabled_at,?1) WHERE space_id=?2",
            params![now, space_id],
        )?;
        insert_change(
            &transaction,
            space_id,
            space_id,
            "space_deleted",
            &name,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn create_user(
        &self,
        auth: &AuthContext,
        username: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<UserRow, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        validate_credentials(username, password)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let id = Uuid::now_v7().to_string();
        let space_id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        let password_hash = hash_password(password)?;
        transaction.execute("INSERT INTO users (id,username,password_hash,is_admin,created_at) VALUES (?1,?2,?3,?4,?5)",
            params![id,username,password_hash,is_admin,now]).map_err(|error| match error {
                rusqlite::Error::SqliteFailure(ref failure,_) if failure.extended_code==2067 => DbError::Conflict("Benutzername ist bereits vergeben".into()),
                other => DbError::from(other),
            })?;
        transaction.execute("INSERT INTO spaces (id,name,kind,owner_user_id,created_at) VALUES (?1,?2,'personal',?3,?4)",
            params![space_id,format!("Dateien von {username}"),id,now])?;
        transaction.execute(
            "INSERT INTO memberships (space_id,user_id,role) VALUES (?1,?2,'owner')",
            params![space_id, id],
        )?;
        transaction.commit()?;
        Ok(UserRow {
            id,
            username: username.into(),
            is_admin,
            created_at: now,
            disabled: false,
            quota_bytes: None,
        })
    }

    pub fn list_users(&self, auth: &AuthContext) -> Result<Vec<UserRow>, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        let mut statement=connection.prepare("SELECT id,username,is_admin,created_at,disabled_at IS NOT NULL,quota_bytes FROM users ORDER BY username COLLATE NOCASE")?;
        Ok(statement
            .query_map([], |row| {
                Ok(UserRow {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    is_admin: row.get(2)?,
                    created_at: row.get(3)?,
                    disabled: row.get(4)?,
                    quota_bytes: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_group(&self, auth: &AuthContext, name: &str) -> Result<GroupRow, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(DbError::Invalid(
                "Gruppenname muss 1 bis 120 Zeichen lang sein".into(),
            ));
        }
        let connection = self.lock()?;
        let id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO groups (id,name,created_at) VALUES (?1,?2,?3)",
                params![id, name, now],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(ref failure, _) if failure.extended_code == 2067 => {
                    DbError::Conflict("Gruppenname ist bereits vergeben".into())
                }
                other => DbError::from(other),
            })?;
        Ok(GroupRow {
            id,
            name: name.into(),
            member_count: 0,
            member_ids: Vec::new(),
            created_at: now,
        })
    }

    pub fn list_admin_spaces(&self, auth: &AuthContext) -> Result<Vec<AdminSpaceRow>, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT s.id,s.name,s.kind,u.username,s.quota_bytes,s.archive_protected,
                    COALESCE((SELECT SUM(f.size) FROM files f WHERE f.space_id=s.id AND f.deleted_at IS NULL),0),
                    COALESCE((SELECT SUM(v.size) FROM versions v JOIN files f ON f.id=v.file_id WHERE f.space_id=s.id),0)
             FROM spaces s JOIN users u ON u.id=s.owner_user_id WHERE s.deleted_at IS NULL
             ORDER BY s.kind,s.name COLLATE NOCASE",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(AdminSpaceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    owner: row.get(3)?,
                    quota_bytes: row.get(4)?,
                    archive_protected: row.get(5)?,
                    current_bytes: row.get(6)?,
                    version_bytes: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_groups(&self, auth: &AuthContext) -> Result<Vec<GroupRow>, DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT g.id,g.name,COUNT(gm.user_id),COALESCE(GROUP_CONCAT(gm.user_id),''),g.created_at FROM groups g
             LEFT JOIN group_members gm ON gm.group_id=g.id GROUP BY g.id ORDER BY g.name COLLATE NOCASE",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(GroupRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    member_count: row.get(2)?,
                    member_ids: row
                        .get::<_, String>(3)?
                        .split(',')
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_group_member(
        &self,
        auth: &AuthContext,
        group_id: &str,
        user_id: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        if enabled {
            connection.execute(
                "INSERT INTO group_members (group_id,user_id) VALUES (?1,?2) ON CONFLICT DO NOTHING",
                params![group_id,user_id],
            ).map_err(|error| match error {
                rusqlite::Error::SqliteFailure(_,_) => DbError::NotFound,
                other => DbError::from(other),
            })?;
        } else {
            connection.execute(
                "DELETE FROM group_members WHERE group_id=?1 AND user_id=?2",
                params![group_id, user_id],
            )?;
        }
        Ok(())
    }

    pub fn set_group_space_role(
        &self,
        auth: &AuthContext,
        space_id: &str,
        group_id: &str,
        role: Option<&str>,
    ) -> Result<(), DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        if let Some(role) = role {
            if !matches!(role, "manager" | "editor" | "viewer" | "uploader") {
                return Err(DbError::Invalid("Ungültige Space-Rolle".into()));
            }
            let connection = self.lock()?;
            connection
                .execute(
                    "INSERT INTO group_space_roles (group_id,space_id,role) VALUES (?1,?2,?3)
                 ON CONFLICT(group_id,space_id) DO UPDATE SET role=excluded.role",
                    params![group_id, space_id, role],
                )
                .map_err(|error| match error {
                    rusqlite::Error::SqliteFailure(_, _) => DbError::NotFound,
                    other => DbError::from(other),
                })?;
        } else {
            let connection = self.lock()?;
            connection.execute(
                "DELETE FROM group_space_roles WHERE group_id=?1 AND space_id=?2",
                params![group_id, space_id],
            )?;
        }
        Ok(())
    }

    pub fn space_access(&self, auth: &AuthContext, space_id: &str) -> Result<SpaceAccess, DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let connection = self.lock()?;
        let members = {
            let mut statement = connection.prepare(
                "SELECT u.id,u.username,m.role FROM memberships m JOIN users u ON u.id=m.user_id
                 WHERE m.space_id=?1 ORDER BY u.username COLLATE NOCASE",
            )?;
            statement
                .query_map([space_id], |row| {
                    Ok(SpaceMemberRow {
                        user_id: row.get(0)?,
                        username: row.get(1)?,
                        role: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let group_roles = {
            let mut statement = connection.prepare(
                "SELECT g.id,g.name,gsr.role FROM group_space_roles gsr JOIN groups g ON g.id=gsr.group_id
                 WHERE gsr.space_id=?1 ORDER BY g.name COLLATE NOCASE",
            )?;
            statement
                .query_map([space_id], |row| {
                    Ok(SpaceGroupRoleRow {
                        group_id: row.get(0)?,
                        name: row.get(1)?,
                        role: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let available_users = {
            let mut statement = connection.prepare(
                "SELECT id,username FROM users WHERE disabled_at IS NULL ORDER BY username COLLATE NOCASE",
            )?;
            statement
                .query_map([], |row| {
                    Ok(AccessChoice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let available_groups = {
            let mut statement =
                connection.prepare("SELECT id,name FROM groups ORDER BY name COLLATE NOCASE")?;
            statement
                .query_map([], |row| {
                    Ok(AccessChoice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(SpaceAccess {
            members,
            group_roles,
            available_users,
            available_groups,
        })
    }

    pub fn set_user_quota(
        &self,
        auth: &AuthContext,
        user_id: &str,
        quota_bytes: Option<i64>,
    ) -> Result<(), DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        validate_quota(quota_bytes)?;
        let connection = self.lock()?;
        if connection.execute(
            "UPDATE users SET quota_bytes=?1 WHERE id=?2",
            params![quota_bytes, user_id],
        )? == 0
        {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    pub fn set_space_quota(
        &self,
        auth: &AuthContext,
        space_id: &str,
        quota_bytes: Option<i64>,
    ) -> Result<(), DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        validate_quota(quota_bytes)?;
        let connection = self.lock()?;
        if connection.execute(
            "UPDATE spaces SET quota_bytes=?1 WHERE id=?2",
            params![quota_bytes, space_id],
        )? == 0
        {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    pub fn set_archive_protection(
        &self,
        auth: &AuthContext,
        space_id: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        if !auth.is_admin {
            return Err(DbError::Forbidden);
        }
        let connection = self.lock()?;
        if connection.execute(
            "UPDATE spaces SET archive_protected=?1 WHERE id=?2",
            params![enabled, space_id],
        )? == 0
        {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    pub fn set_editor_sharing(
        &self,
        auth: &AuthContext,
        space_id: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        require_permission_connection(&connection, space_id, &auth.user_id, Permission::Manage)?;
        if connection.execute(
            "UPDATE spaces SET editor_sharing=?1 WHERE id=?2 AND deleted_at IS NULL",
            params![enabled, space_id],
        )? == 0
        {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    pub fn space_usage(&self, auth: &AuthContext, space_id: &str) -> Result<StorageUsage, DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::View)?;
        let connection = self.lock()?;
        let (current_bytes, version_bytes, quota_bytes): (i64, i64, Option<i64>) = connection.query_row(
            "SELECT
               COALESCE((SELECT SUM(size) FROM files WHERE space_id=?1 AND deleted_at IS NULL),0),
               COALESCE((SELECT SUM(v.size) FROM versions v JOIN files f ON f.id=v.file_id WHERE f.space_id=?1),0),
               quota_bytes FROM spaces WHERE id=?1",
            [space_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?.ok_or(DbError::NotFound)?;
        let physical_bytes: i64 = connection.query_row(
            "SELECT COALESCE(SUM(size),0) FROM blocks WHERE space_id=?1 AND reference_count>0",
            [space_id],
            |row| row.get(0),
        )?;
        Ok(StorageUsage {
            current_bytes,
            version_bytes,
            physical_bytes,
            quota_bytes,
        })
    }

    pub fn set_member(
        &self,
        auth: &AuthContext,
        space_id: &str,
        user_id: &str,
        role: &str,
    ) -> Result<(), DbError> {
        if !matches!(role, "manager" | "editor" | "viewer" | "uploader") {
            return Err(DbError::Invalid("Ungültige Space-Rolle".into()));
        }
        let actor_role = self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let connection = self.lock()?;
        let target_exists = connection
            .query_row(
                "SELECT 1 FROM users WHERE id=?1 AND disabled_at IS NULL",
                [user_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !target_exists {
            return Err(DbError::NotFound);
        }
        let target_role: Option<String> = connection
            .query_row(
                "SELECT role FROM memberships WHERE space_id=?1 AND user_id=?2",
                params![space_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        if target_role.as_deref() == Some("owner") {
            return Err(DbError::Forbidden);
        }
        if actor_role == "manager" && role == "manager" {
            return Err(DbError::Forbidden);
        }
        connection.execute(
            "INSERT INTO memberships (space_id,user_id,role) VALUES (?1,?2,?3)
            ON CONFLICT(space_id,user_id) DO UPDATE SET role=excluded.role",
            params![space_id, user_id, role],
        )?;
        Ok(())
    }

    pub fn remove_member(
        &self,
        auth: &AuthContext,
        space_id: &str,
        user_id: &str,
    ) -> Result<(), DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Manage)?;
        let connection = self.lock()?;
        let target_role: Option<String> = connection
            .query_row(
                "SELECT role FROM memberships WHERE space_id=?1 AND user_id=?2",
                params![space_id, user_id],
                |row| row.get(0),
            )
            .optional()?;
        if target_role.as_deref() == Some("owner") {
            return Err(DbError::Forbidden);
        }
        connection.execute(
            "DELETE FROM memberships WHERE space_id=?1 AND user_id=?2",
            params![space_id, user_id],
        )?;
        Ok(())
    }

    pub fn require_permission(
        &self,
        space_id: &str,
        user_id: &str,
        permission: Permission,
    ) -> Result<String, DbError> {
        let connection = self.lock()?;
        require_permission_connection(&connection, space_id, user_id, permission)
    }

    pub fn list_files(
        &self,
        space_id: &str,
        user_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<FileRow>, DbError> {
        self.require_permission(space_id, user_id, Permission::View)?;
        let connection = self.lock()?;
        let sql = if include_deleted {
            "SELECT id,path,size,current_version_id,updated_at,deleted_at FROM files WHERE space_id=?1 ORDER BY path"
        } else {
            "SELECT id,path,size,current_version_id,updated_at,deleted_at FROM files WHERE space_id=?1 AND deleted_at IS NULL ORDER BY path"
        };
        let mut statement = connection.prepare(sql)?;
        Ok(statement
            .query_map([space_id], |row| {
                Ok(FileRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    size: row.get(2)?,
                    current_version_id: row.get(3)?,
                    updated_at: row.get(4)?,
                    deleted_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_directories(
        &self,
        space_id: &str,
        user_id: &str,
    ) -> Result<Vec<DirectoryRow>, DbError> {
        self.require_permission(space_id, user_id, Permission::View)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id,path,created_at FROM directories WHERE space_id=?1 AND deleted_at IS NULL ORDER BY path",
        )?;
        Ok(statement
            .query_map([space_id], |row| {
                Ok(DirectoryRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn create_directory(
        &self,
        auth: &AuthContext,
        space_id: &str,
        path: &str,
    ) -> Result<DirectoryRow, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        require_permission_connection(&transaction, space_id, &auth.user_id, Permission::Edit)?;
        if file_ancestor_exists(&transaction, space_id, path)? {
            return Err(DbError::Conflict(
                "Eine Datei im Zielpfad verhindert das Anlegen des Ordners".into(),
            ));
        }
        let occupied_file = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, path],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if occupied_file {
            return Err(DbError::Conflict(
                "Am Ordnerpfad existiert bereits eine Datei".into(),
            ));
        }
        let id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        ensure_parent_directories(&transaction, space_id, path, &now)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO directories (id,space_id,path,created_at) VALUES (?1,?2,?3,?4)",
            params![id, space_id, path, now],
        )?;
        if inserted == 0 {
            return Err(DbError::Conflict("Dieser Ordner existiert bereits".into()));
        }
        insert_change(
            &transaction,
            space_id,
            &id,
            "folder_created",
            path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(DirectoryRow {
            id,
            path: path.into(),
            created_at: now,
        })
    }

    pub fn delete_directory(
        &self,
        auth: &AuthContext,
        directory_id: &str,
    ) -> Result<usize, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, path, archive_protected): (String, String, bool) = transaction
            .query_row(
                "SELECT d.space_id,d.path,s.archive_protected FROM directories d JOIN spaces s ON s.id=d.space_id
                 WHERE d.id=?1 AND d.deleted_at IS NULL",
                [directory_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if archive_protected {
            return Err(DbError::Conflict(
                "Archivschutz verhindert das Löschen dieses Ordners".into(),
            ));
        }
        let escaped_path = escape_like(&path);
        let prefix = format!("{escaped_path}/%");
        let affected: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT id,path FROM files WHERE space_id=?1 AND deleted_at IS NULL AND path LIKE ?2 ESCAPE '\\'",
            )?;
            statement
                .query_map(params![space_id, prefix], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let now = Utc::now().to_rfc3339();
        for (file_id, file_path) in &affected {
            transaction.execute(
                "UPDATE files SET deleted_at=?1,updated_at=?1 WHERE id=?2",
                params![now, file_id],
            )?;
            insert_change(
                &transaction,
                &space_id,
                file_id,
                "deleted",
                file_path,
                None,
                auth,
                &now,
            )?;
        }
        transaction.execute(
            "UPDATE directories SET deleted_at=?1 WHERE space_id=?2 AND deleted_at IS NULL AND (path=?3 OR path LIKE ?4 ESCAPE '\\')",
            params![now, space_id, path, prefix],
        )?;
        insert_change(
            &transaction,
            &space_id,
            directory_id,
            "folder_deleted",
            &path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(affected.len())
    }

    pub fn move_directory(
        &self,
        auth: &AuthContext,
        directory_id: &str,
        new_path: &str,
    ) -> Result<DirectoryRow, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, old_path, created_at): (String, String, String) = transaction
            .query_row(
                "SELECT space_id,path,created_at FROM directories WHERE id=?1 AND deleted_at IS NULL",
                [directory_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if old_path == new_path {
            return Ok(DirectoryRow {
                id: directory_id.into(),
                path: old_path,
                created_at,
            });
        }
        if new_path.starts_with(&format!("{old_path}/")) {
            return Err(DbError::Conflict(
                "Ein Ordner kann nicht in sich selbst verschoben werden".into(),
            ));
        }

        let source_prefix = format!("{old_path}/");
        let source_directories: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT id,path FROM directories WHERE space_id=?1 AND deleted_at IS NULL AND (path=?2 OR path LIKE ?3 ESCAPE '\\') ORDER BY length(path)",
            )?;
            statement
                .query_map(
                    params![
                        space_id,
                        old_path,
                        format!("{}%", escape_like(&source_prefix))
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let source_files: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT id,path FROM files WHERE space_id=?1 AND deleted_at IS NULL AND path LIKE ?2 ESCAPE '\\' ORDER BY path",
            )?;
            statement
                .query_map(
                    params![space_id, format!("{}%", escape_like(&source_prefix))],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        let source_directory_ids: HashSet<_> = source_directories
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        let source_file_ids: HashSet<_> = source_files.iter().map(|(id, _)| id.as_str()).collect();
        let occupied_directories: HashSet<String> = {
            let mut statement = transaction.prepare(
                "SELECT id,path FROM directories WHERE space_id=?1 AND deleted_at IS NULL",
            )?;
            statement
                .query_map([&space_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(id, _)| !source_directory_ids.contains(id.as_str()))
                .map(|(_, path)| path)
                .collect()
        };
        let occupied_files: HashSet<String> = {
            let mut statement = transaction
                .prepare("SELECT id,path FROM files WHERE space_id=?1 AND deleted_at IS NULL")?;
            statement
                .query_map([&space_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(id, _)| !source_file_ids.contains(id.as_str()))
                .map(|(_, path)| path)
                .collect()
        };
        let rewrite = |path: &str| {
            if path == old_path {
                new_path.to_owned()
            } else {
                format!("{new_path}/{}", &path[source_prefix.len()..])
            }
        };
        let target_directories: Vec<_> = source_directories
            .iter()
            .map(|(id, path)| (id.as_str(), rewrite(path)))
            .collect();
        let target_files: Vec<_> = source_files
            .iter()
            .map(|(id, path)| (id.as_str(), rewrite(path)))
            .collect();
        if target_directories
            .iter()
            .any(|(_, path)| occupied_directories.contains(path) || occupied_files.contains(path))
            || target_files.iter().any(|(_, path)| {
                occupied_files.contains(path) || occupied_directories.contains(path)
            })
        {
            return Err(DbError::Conflict(
                "Am Ziel existiert bereits eine Datei oder ein Ordner".into(),
            ));
        }

        let now = Utc::now().to_rfc3339();
        if file_ancestor_exists(&transaction, &space_id, new_path)? {
            return Err(DbError::Conflict(
                "Eine Datei im Zielpfad verhindert das Verschieben des Ordners".into(),
            ));
        }
        ensure_parent_directories(&transaction, &space_id, new_path, &now)?;
        for (id, path) in &target_directories {
            transaction.execute(
                "UPDATE directories SET path=?1 WHERE id=?2",
                params![path, id],
            )?;
        }
        for (id, path) in &target_files {
            transaction.execute(
                "UPDATE files SET path=?1,updated_at=?2 WHERE id=?3",
                params![path, now, id],
            )?;
            insert_change(&transaction, &space_id, id, "moved", path, None, auth, &now)?;
        }
        insert_change(
            &transaction,
            &space_id,
            directory_id,
            "folder_moved",
            new_path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(DirectoryRow {
            id: directory_id.into(),
            path: new_path.into(),
            created_at,
        })
    }

    pub fn create_upload(
        &self,
        auth: &AuthContext,
        space_id: &str,
        path: &str,
        file_id: Option<&str>,
        base_version_id: Option<&str>,
        size: i64,
        blocks: &[StoredBlock],
        idempotency_key: &str,
    ) -> Result<UploadRow, DbError> {
        let connection = self.lock()?;
        create_upload_on(
            &connection,
            auth,
            space_id,
            &NewUpload {
                path: path.into(),
                file_id: file_id.map(str::to_owned),
                base_version_id: base_version_id.map(str::to_owned),
                size,
                blocks: blocks.to_vec(),
                idempotency_key: idempotency_key.into(),
            },
        )
    }

    /// Persists a group of resumable upload manifests in one FULL-synchronous transaction.
    /// A power loss therefore leaves either all manifests visible or none of them, while avoiding
    /// one disk barrier per small file.
    pub fn create_uploads(
        &self,
        auth: &AuthContext,
        space_id: &str,
        uploads: &[NewUpload],
    ) -> Result<Vec<UploadRow>, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let mut created = Vec::with_capacity(uploads.len());
        for upload in uploads {
            created.push(create_upload_on(&transaction, auth, space_id, upload)?);
        }
        transaction.commit()?;
        Ok(created)
    }

    pub fn get_upload(&self, upload_id: &str, auth: &AuthContext) -> Result<UploadRow, DbError> {
        let connection = self.lock()?;
        let upload = load_upload(&connection, upload_id)?.ok_or(DbError::NotFound)?;
        if upload.user_id != auth.user_id || upload.device_id != auth.device_id {
            return Err(DbError::Forbidden);
        }
        if upload.status == "aborted"
            || (upload.status == "open" && upload.expires_at <= Utc::now().to_rfc3339())
        {
            return Err(DbError::Conflict("Upload ist nicht mehr offen".into()));
        }
        Ok(upload)
    }

    pub fn commit_upload(
        &self,
        upload_id: &str,
        auth: &AuthContext,
    ) -> Result<CommitResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let upload = load_upload(&transaction, upload_id)?.ok_or(DbError::NotFound)?;
        if upload.user_id != auth.user_id || upload.device_id != auth.device_id {
            return Err(DbError::Forbidden);
        }
        if upload.status == "committed" {
            let json: String = transaction.query_row(
                "SELECT response_json FROM idempotency_results WHERE device_id=?1 AND idempotency_key=?2",
                params![auth.device_id, upload.idempotency_key], |row| row.get(0),
            )?;
            return Ok(serde_json::from_str(&json)?);
        }
        if upload.status != "open" || upload.expires_at <= Utc::now().to_rfc3339() {
            return Err(DbError::Conflict(
                "Upload ist abgelaufen oder geschlossen".into(),
            ));
        }
        let role = require_permission_connection(
            &transaction,
            &upload.space_id,
            &auth.user_id,
            Permission::Upload,
        )?;
        enforce_quotas(&transaction, &upload.space_id, upload.size)?;

        let now = Utc::now();
        let existing = find_target_file(&transaction, &upload)?;
        let (file_id, final_path, conflict) = match existing {
            Some((id, current_version, existing_path)) => {
                if role == "uploader" {
                    (
                        Uuid::now_v7().to_string(),
                        available_upload_path(&transaction, &upload.space_id, &upload.path)?,
                        true,
                    )
                } else {
                    let same_base = upload.base_version_id.as_deref() == current_version.as_deref();
                    let targets_same_file = upload
                        .file_id
                        .as_deref()
                        .is_none_or(|candidate| candidate == id);
                    if same_base && targets_same_file {
                        (id, existing_path, false)
                    } else {
                        (
                            Uuid::now_v7().to_string(),
                            conflict_path(&upload.path, &auth.device_name, now),
                            true,
                        )
                    }
                }
            }
            None if upload.file_id.is_some() => (
                Uuid::now_v7().to_string(),
                conflict_path(&upload.path, &auth.device_name, now),
                true,
            ),
            None => (Uuid::now_v7().to_string(), upload.path.clone(), false),
        };

        let now_text = now.to_rfc3339();
        ensure_file_target(&transaction, &upload.space_id, &final_path)?;
        ensure_parent_directories(&transaction, &upload.space_id, &final_path, &now_text)?;
        if transaction
            .query_row("SELECT 1 FROM files WHERE id=?1", [&file_id], |_| Ok(()))
            .optional()?
            .is_none()
        {
            transaction.execute(
                "INSERT INTO files (id,space_id,path,size,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
                params![file_id, upload.space_id, final_path, upload.size, now_text],
            )?;
        }
        let version_id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO versions (id,file_id,size,created_by_user_id,created_by_device_id,created_at,format)
             VALUES (?1,?2,?3,?4,?5,?6,1)",
            params![version_id,file_id,upload.size,auth.user_id,auth.device_id,now_text],
        )?;
        for (position, block) in upload.blocks.iter().enumerate() {
            transaction.execute(
                "INSERT INTO blocks (space_id,hash,size,reference_count,created_at) VALUES (?1,?2,?3,1,?4)
                 ON CONFLICT(space_id,hash) DO UPDATE SET reference_count=reference_count+1",
                params![upload.space_id, block.hash, block.size, now_text],
            )?;
            transaction.execute(
                "INSERT INTO version_blocks (version_id,position,block_hash,block_size) VALUES (?1,?2,?3,?4)",
                params![version_id, position as i64, block.hash, block.size],
            )?;
        }
        transaction.execute(
            "UPDATE files SET path=?1,size=?2,current_version_id=?3,updated_at=?4,deleted_at=NULL WHERE id=?5",
            params![final_path,upload.size,version_id,now_text,file_id],
        )?;
        let event_id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO changes (event_id,space_id,object_id,kind,path,version_id,user_id,device_id,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![event_id,upload.space_id,file_id,if conflict {"conflict_created"} else {"content_changed"},final_path,version_id,auth.user_id,auth.device_id,now_text],
        )?;
        let sequence = transaction.last_insert_rowid();
        let result = CommitResult {
            file_id,
            version_id,
            path: final_path,
            conflict,
            change_sequence: sequence,
        };
        transaction.execute(
            "UPDATE uploads SET status='committed' WHERE id=?1",
            [&upload.id],
        )?;
        transaction.execute(
            "INSERT INTO idempotency_results (device_id,idempotency_key,response_json,created_at) VALUES (?1,?2,?3,?4)",
            params![auth.device_id,upload.idempotency_key,serde_json::to_string(&result)?,now_text],
        )?;
        transaction.commit()?;
        Ok(result)
    }

    /// Commits a group of fully uploaded files atomically. The existing single-file endpoint is
    /// kept for compatibility and retries; sync clients use this path to share one durable SQLite
    /// barrier across many small files.
    pub fn commit_uploads(
        &self,
        upload_ids: &[String],
        auth: &AuthContext,
    ) -> Result<Vec<CommitResult>, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let mut results = Vec::with_capacity(upload_ids.len());
        for upload_id in upload_ids {
            results.push(commit_upload_on(&transaction, upload_id, auth)?);
        }
        transaction.commit()?;
        Ok(results)
    }

    pub fn file_blocks(
        &self,
        file_id: &str,
        user_id: &str,
    ) -> Result<(String, String, Vec<StoredBlock>), DbError> {
        let connection = self.lock()?;
        let (space_id, name, version_id): (String,String,String) = connection.query_row(
            "SELECT space_id,path,current_version_id FROM files WHERE id=?1 AND deleted_at IS NULL",
            [file_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?.ok_or(DbError::NotFound)?;
        require_permission_connection(&connection, &space_id, user_id, Permission::View)?;
        let mut statement = connection.prepare(
            "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
        )?;
        let blocks = statement
            .query_map([version_id], |row| {
                Ok(StoredBlock {
                    hash: row.get(0)?,
                    size: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((space_id, name, blocks))
    }

    pub fn file_version_blocks(
        &self,
        file_id: &str,
        version_id: &str,
        user_id: &str,
    ) -> Result<(String, String, Vec<StoredBlock>), DbError> {
        let connection = self.lock()?;
        let (space_id, path): (String, String) = connection
            .query_row(
                "SELECT space_id,path FROM files WHERE id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&connection, &space_id, user_id, Permission::View)?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM versions WHERE id=?1 AND file_id=?2",
                params![version_id, file_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(DbError::NotFound);
        }
        let mut statement = connection.prepare(
            "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
        )?;
        let blocks = statement
            .query_map([version_id], |row| {
                Ok(StoredBlock {
                    hash: row.get(0)?,
                    size: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((space_id, path, blocks))
    }

    pub fn list_versions(&self, file_id: &str, user_id: &str) -> Result<Vec<VersionRow>, DbError> {
        let connection = self.lock()?;
        let (space_id, current): (String, Option<String>) = connection
            .query_row(
                "SELECT space_id,current_version_id FROM files WHERE id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&connection, &space_id, user_id, Permission::View)?;
        let mut statement = connection.prepare(
            "SELECT v.id,v.size,u.username,d.name,v.created_at,v.pinned_name
             FROM versions v JOIN users u ON u.id=v.created_by_user_id JOIN devices d ON d.id=v.created_by_device_id
             WHERE v.file_id=?1 ORDER BY v.created_at DESC",
        )?;
        Ok(statement
            .query_map([file_id], |row| {
                let id: String = row.get(0)?;
                Ok(VersionRow {
                    current: current.as_deref() == Some(id.as_str()),
                    id,
                    size: row.get(1)?,
                    created_by: row.get(2)?,
                    device_name: row.get(3)?,
                    created_at: row.get(4)?,
                    pinned_name: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn restore_version(
        &self,
        file_id: &str,
        source_version_id: &str,
        auth: &AuthContext,
    ) -> Result<CommitResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, path, deleted_at): (String, String, Option<String>) = transaction
            .query_row(
                "SELECT space_id,path,deleted_at FROM files WHERE id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if deleted_at.is_some() {
            return Err(DbError::Conflict(
                "Datei muss vor der Versionswiederherstellung aus dem Papierkorb geholt werden"
                    .into(),
            ));
        }
        let size: i64 = transaction
            .query_row(
                "SELECT size FROM versions WHERE id=?1 AND file_id=?2",
                params![source_version_id, file_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        enforce_quotas(&transaction, &space_id, size)?;
        let blocks = {
            let mut statement = transaction.prepare(
                "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
            )?;
            statement
                .query_map([source_version_id], |row| {
                    Ok(StoredBlock {
                        hash: row.get(0)?,
                        size: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let version_id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO versions (id,file_id,size,created_by_user_id,created_by_device_id,created_at,format)
             VALUES (?1,?2,?3,?4,?5,?6,1)",
            params![version_id,file_id,size,auth.user_id,auth.device_id,now],
        )?;
        for (position, block) in blocks.iter().enumerate() {
            transaction.execute(
                "INSERT INTO version_blocks (version_id,position,block_hash,block_size) VALUES (?1,?2,?3,?4)",
                params![version_id,position as i64,block.hash,block.size],
            )?;
            transaction.execute(
                "UPDATE blocks SET reference_count=reference_count+1 WHERE space_id=?1 AND hash=?2",
                params![space_id, block.hash],
            )?;
        }
        transaction.execute(
            "UPDATE files SET size=?1,current_version_id=?2,updated_at=?3 WHERE id=?4",
            params![size, version_id, now, file_id],
        )?;
        let sequence = insert_change(
            &transaction,
            &space_id,
            file_id,
            "version_restored",
            &path,
            Some(&version_id),
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(CommitResult {
            file_id: file_id.into(),
            version_id,
            path,
            conflict: false,
            change_sequence: sequence,
        })
    }

    pub fn pin_version(
        &self,
        file_id: &str,
        version_id: &str,
        name: Option<&str>,
        auth: &AuthContext,
    ) -> Result<(), DbError> {
        let connection = self.lock()?;
        let space_id = connection
            .query_row("SELECT space_id FROM files WHERE id=?1", [file_id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&connection, &space_id, &auth.user_id, Permission::Edit)?;
        let name = name.map(str::trim).filter(|value| !value.is_empty());
        if name.is_some_and(|value| value.chars().count() > 120) {
            return Err(DbError::Invalid("Versionsname ist zu lang".into()));
        }
        let changed = connection.execute(
            "UPDATE versions SET pinned_name=?1 WHERE id=?2 AND file_id=?3",
            params![name, version_id, file_id],
        )?;
        if changed == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    pub fn move_file(
        &self,
        file_id: &str,
        new_path: &str,
        auth: &AuthContext,
    ) -> Result<MutationResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, old_path, deleted_at): (String, String, Option<String>) = transaction
            .query_row(
                "SELECT space_id,path,deleted_at FROM files WHERE id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if deleted_at.is_some() {
            return Err(DbError::Conflict("Datei liegt im Papierkorb".into()));
        }
        if old_path == new_path {
            return Ok(MutationResult {
                file_id: file_id.into(),
                path: old_path,
                change_sequence: None,
            });
        }
        let occupied = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, new_path],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if occupied {
            return Err(DbError::Conflict(
                "Am Ziel existiert bereits eine Datei".into(),
            ));
        }
        ensure_file_target(&transaction, &space_id, new_path)?;
        let now = Utc::now().to_rfc3339();
        ensure_parent_directories(&transaction, &space_id, new_path, &now)?;
        transaction.execute(
            "UPDATE files SET path=?1,updated_at=?2 WHERE id=?3",
            params![new_path, now, file_id],
        )?;
        let sequence = insert_change(
            &transaction,
            &space_id,
            file_id,
            "moved",
            new_path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(MutationResult {
            file_id: file_id.into(),
            path: new_path.into(),
            change_sequence: Some(sequence),
        })
    }

    pub fn copy_file(
        &self,
        file_id: &str,
        target_path: &str,
        auth: &AuthContext,
    ) -> Result<CommitResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, source_version_id, size): (String, String, i64) = transaction
            .query_row(
                "SELECT space_id,current_version_id,size FROM files WHERE id=?1 AND deleted_at IS NULL",
                [file_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).optional()?.ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        enforce_quotas(&transaction, &space_id, size)?;
        let occupied = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, target_path],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if occupied {
            return Err(DbError::Conflict(
                "Am Ziel existiert bereits eine Datei".into(),
            ));
        }
        ensure_file_target(&transaction, &space_id, target_path)?;
        let blocks: Vec<StoredBlock> = {
            let mut statement = transaction.prepare(
                "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
            )?;
            statement
                .query_map([source_version_id], |row| {
                    Ok(StoredBlock {
                        hash: row.get(0)?,
                        size: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let new_file_id = Uuid::now_v7().to_string();
        let version_id = Uuid::now_v7().to_string();
        let now = Utc::now().to_rfc3339();
        ensure_parent_directories(&transaction, &space_id, target_path, &now)?;
        transaction.execute(
            "INSERT INTO files (id,space_id,path,size,current_version_id,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![new_file_id, space_id, target_path, size, version_id, now],
        )?;
        transaction.execute(
            "INSERT INTO versions (id,file_id,size,created_by_user_id,created_by_device_id,created_at,format)
             VALUES (?1,?2,?3,?4,?5,?6,1)",
            params![version_id,new_file_id,size,auth.user_id,auth.device_id,now],
        )?;
        for (position, block) in blocks.iter().enumerate() {
            transaction.execute(
                "INSERT INTO version_blocks (version_id,position,block_hash,block_size) VALUES (?1,?2,?3,?4)",
                params![version_id,position as i64,block.hash,block.size],
            )?;
            transaction.execute(
                "UPDATE blocks SET reference_count=reference_count+1 WHERE space_id=?1 AND hash=?2",
                params![space_id, block.hash],
            )?;
        }
        let sequence = insert_change(
            &transaction,
            &space_id,
            &new_file_id,
            "created",
            target_path,
            Some(&version_id),
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(CommitResult {
            file_id: new_file_id,
            version_id,
            path: target_path.into(),
            conflict: false,
            change_sequence: sequence,
        })
    }

    pub fn delete_file(
        &self,
        file_id: &str,
        auth: &AuthContext,
    ) -> Result<MutationResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, path, deleted_at, archive_protected): (String, String, Option<String>, bool) = transaction
            .query_row(
                "SELECT f.space_id,f.path,f.deleted_at,s.archive_protected FROM files f JOIN spaces s ON s.id=f.space_id WHERE f.id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if archive_protected {
            return Err(DbError::Forbidden);
        }
        if deleted_at.is_some() {
            return Ok(MutationResult {
                file_id: file_id.into(),
                path,
                change_sequence: None,
            });
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE files SET deleted_at=?1,updated_at=?1 WHERE id=?2",
            params![now, file_id],
        )?;
        let sequence = insert_change(
            &transaction,
            &space_id,
            file_id,
            "deleted",
            &path,
            None,
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(MutationResult {
            file_id: file_id.into(),
            path,
            change_sequence: Some(sequence),
        })
    }

    pub fn restore_file(
        &self,
        file_id: &str,
        auth: &AuthContext,
    ) -> Result<MutationResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let (space_id, original_path, deleted_at): (String, String, Option<String>) = transaction
            .query_row(
                "SELECT space_id,path,deleted_at FROM files WHERE id=?1",
                [file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        require_permission_connection(&transaction, &space_id, &auth.user_id, Permission::Edit)?;
        if deleted_at.is_none() {
            return Ok(MutationResult {
                file_id: file_id.into(),
                path: original_path,
                change_sequence: None,
            });
        }
        let occupied = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, original_path],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let path = if occupied {
            available_restore_path(&transaction, &space_id, &original_path)?
        } else {
            original_path
        };
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE files SET path=?1,deleted_at=NULL,updated_at=?2 WHERE id=?3",
            params![path, now, file_id],
        )?;
        let version_id: Option<String> = transaction.query_row(
            "SELECT current_version_id FROM files WHERE id=?1",
            [file_id],
            |row| row.get(0),
        )?;
        let sequence = insert_change(
            &transaction,
            &space_id,
            file_id,
            "restored",
            &path,
            version_id.as_deref(),
            auth,
            &now,
        )?;
        transaction.commit()?;
        Ok(MutationResult {
            file_id: file_id.into(),
            path,
            change_sequence: Some(sequence),
        })
    }

    pub fn list_changes(
        &self,
        user_id: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<ChangeRow>, DbError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT c.sequence,c.event_id,c.space_id,c.object_id,c.kind,c.path,c.version_id,c.created_at
             FROM changes c
             WHERE c.sequence>?2 AND (
               EXISTS (SELECT 1 FROM memberships m WHERE m.space_id=c.space_id AND m.user_id=?1
                       AND m.role IN ('viewer','editor','manager','owner'))
               OR EXISTS (SELECT 1 FROM group_space_roles gsr JOIN group_members gm ON gm.group_id=gsr.group_id
                          WHERE gsr.space_id=c.space_id AND gm.user_id=?1
                          AND gsr.role IN ('viewer','editor','manager','owner'))
             ) ORDER BY c.sequence LIMIT ?3",
        )?;
        Ok(statement
            .query_map(params![user_id, after, limit.min(1000) as i64], |row| {
                Ok(ChangeRow {
                    sequence: row.get(0)?,
                    event_id: row.get(1)?,
                    space_id: row.get(2)?,
                    object_id: row.get(3)?,
                    kind: row.get(4)?,
                    path: row.get(5)?,
                    version_id: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn folder_restore_preview(
        &self,
        auth: &AuthContext,
        space_id: &str,
        folder_path: &str,
        target_at: &str,
    ) -> Result<FolderRestorePreview, DbError> {
        self.require_permission(space_id, &auth.user_id, Permission::Edit)?;
        let connection = self.lock()?;
        build_folder_restore_preview(&connection, space_id, folder_path, target_at)
    }

    pub fn restore_folder_at(
        &self,
        auth: &AuthContext,
        space_id: &str,
        folder_path: &str,
        target_at: &str,
    ) -> Result<FolderRestoreResult, DbError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        require_permission_connection(&transaction, space_id, &auth.user_id, Permission::Edit)?;
        let preview = build_folder_restore_preview(&transaction, space_id, folder_path, target_at)?;
        if !preview.unavailable.is_empty() {
            return Err(DbError::Conflict(format!(
                "Der Ordnerstand ist unvollständig: {}",
                preview.unavailable.join(", ")
            )));
        }
        let destructive = preview
            .actions
            .iter()
            .any(|action| action.action == "delete");
        let archive_protected: bool = transaction.query_row(
            "SELECT archive_protected FROM spaces WHERE id=?1",
            [space_id],
            |row| row.get(0),
        )?;
        if destructive && archive_protected {
            return Err(DbError::Conflict(
                "Archivschutz verhindert das Entfernen neuerer Ordnerinhalte".into(),
            ));
        }
        let desired = historical_states_at(&transaction, space_id, target_at)?;
        let desired = desired
            .into_iter()
            .filter(|state| !state.deleted && path_is_inside(&state.path, folder_path))
            .map(|state| (state.file_id.clone(), state))
            .collect::<HashMap<_, _>>();
        let now = Utc::now().to_rfc3339();
        let action_ids = preview
            .actions
            .iter()
            .filter(|action| action.action != "delete")
            .map(|action| action.file_id.as_str())
            .collect::<HashSet<_>>();

        // Erst alle betroffenen aktuellen Pfade freigeben, damit auch historische
        // Namensvertauschungen atomar und ohne Zwischenkollision wiederherstellbar sind.
        for action in &preview.actions {
            if action.action == "delete" || action_ids.contains(action.file_id.as_str()) {
                transaction.execute(
                    "UPDATE files SET deleted_at=?1,updated_at=?1 WHERE id=?2",
                    params![now, action.file_id],
                )?;
            }
        }

        let mut restored_files = 0;
        let mut deleted_files = 0;
        for action in &preview.actions {
            if action.action == "delete" {
                insert_change(
                    &transaction,
                    space_id,
                    &action.file_id,
                    "deleted",
                    &action.path,
                    None,
                    auth,
                    &now,
                )?;
                deleted_files += 1;
                continue;
            }
            let state = desired
                .get(&action.file_id)
                .ok_or_else(|| DbError::Conflict("Historischer Dateistand fehlt".into()))?;
            let source_version = state
                .version_id
                .as_deref()
                .ok_or_else(|| DbError::Conflict("Historische Version fehlt".into()))?;
            let size: i64 = transaction
                .query_row(
                    "SELECT size FROM versions WHERE id=?1 AND file_id=?2",
                    params![source_version, state.file_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| {
                    DbError::Conflict(format!(
                        "Version für {} wurde bereits bereinigt",
                        state.path
                    ))
                })?;
            enforce_quotas(&transaction, space_id, size)?;
            let blocks: Vec<StoredBlock> = {
                let mut statement = transaction.prepare(
                    "SELECT block_hash,block_size FROM version_blocks WHERE version_id=?1 ORDER BY position",
                )?;
                statement
                    .query_map([source_version], |row| {
                        Ok(StoredBlock {
                            hash: row.get(0)?,
                            size: row.get(1)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?
            };
            let version_id = Uuid::now_v7().to_string();
            transaction.execute(
                "INSERT INTO versions (id,file_id,size,created_by_user_id,created_by_device_id,created_at,format)
                 VALUES (?1,?2,?3,?4,?5,?6,1)",
                params![version_id,state.file_id,size,auth.user_id,auth.device_id,now],
            )?;
            for (position, block) in blocks.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO version_blocks (version_id,position,block_hash,block_size) VALUES (?1,?2,?3,?4)",
                    params![version_id,position as i64,block.hash,block.size],
                )?;
                transaction.execute(
                    "UPDATE blocks SET reference_count=reference_count+1 WHERE space_id=?1 AND hash=?2",
                    params![space_id, block.hash],
                )?;
            }
            ensure_parent_directories(&transaction, space_id, &state.path, &now)?;
            transaction.execute(
                "UPDATE files SET path=?1,size=?2,current_version_id=?3,updated_at=?4,deleted_at=NULL WHERE id=?5",
                params![state.path,size,version_id,now,state.file_id],
            )?;
            insert_change(
                &transaction,
                space_id,
                &state.file_id,
                "folder_restored",
                &state.path,
                Some(&version_id),
                auth,
                &now,
            )?;
            restored_files += 1;
        }
        transaction.commit()?;
        Ok(FolderRestoreResult {
            restored_files,
            deleted_files,
        })
    }
}

fn historical_states_at(
    connection: &Connection,
    space_id: &str,
    target_at: &str,
) -> Result<Vec<HistoricalFileState>, DbError> {
    let mut statement = connection.prepare(
        "SELECT fs.file_id,fs.path,fs.version_id,fs.deleted
         FROM file_states fs JOIN (
           SELECT file_id,MAX(id) AS latest_id FROM file_states
           WHERE space_id=?1 AND recorded_at<=?2 GROUP BY file_id
         ) latest ON latest.latest_id=fs.id",
    )?;
    Ok(statement
        .query_map(params![space_id, target_at], |row| {
            Ok(HistoricalFileState {
                file_id: row.get(0)?,
                path: row.get(1)?,
                version_id: row.get(2)?,
                deleted: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn build_folder_restore_preview(
    connection: &Connection,
    space_id: &str,
    folder_path: &str,
    target_at: &str,
) -> Result<FolderRestorePreview, DbError> {
    let desired_all = historical_states_at(connection, space_id, target_at)?;
    let desired = desired_all
        .into_iter()
        .filter(|state| !state.deleted && path_is_inside(&state.path, folder_path))
        .map(|state| (state.file_id.clone(), state))
        .collect::<HashMap<_, _>>();
    let current: Vec<(String, String, Option<String>, bool, String)> = {
        let mut statement = connection.prepare(
            "SELECT id,path,current_version_id,deleted_at IS NOT NULL,created_at FROM files WHERE space_id=?1",
        )?;
        statement
            .query_map([space_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let current_by_id = current
        .iter()
        .map(|row| (row.0.as_str(), row))
        .collect::<HashMap<_, _>>();
    let mut actions = Vec::new();
    let mut unavailable = Vec::new();
    for state in desired.values() {
        let Some(version_id) = state.version_id.as_deref() else {
            unavailable.push(format!("{}: keine Version", state.path));
            continue;
        };
        let version_exists = connection
            .query_row(
                "SELECT 1 FROM versions WHERE id=?1 AND file_id=?2",
                params![version_id, state.file_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let Some(current_file) = current_by_id.get(state.file_id.as_str()) else {
            unavailable.push(format!("{}: Datei wurde endgültig bereinigt", state.path));
            continue;
        };
        if !version_exists {
            unavailable.push(format!("{}: Version wurde bereits bereinigt", state.path));
        } else if current_file.3 {
            actions.push(FolderRestoreAction {
                file_id: state.file_id.clone(),
                path: state.path.clone(),
                action: "restore".into(),
            });
        } else if current_file.1 != state.path {
            actions.push(FolderRestoreAction {
                file_id: state.file_id.clone(),
                path: state.path.clone(),
                action: "move".into(),
            });
        } else if current_file.2.as_deref() != Some(version_id) {
            actions.push(FolderRestoreAction {
                file_id: state.file_id.clone(),
                path: state.path.clone(),
                action: "restore_version".into(),
            });
        }
    }
    for current_file in &current {
        if !current_file.3
            && path_is_inside(&current_file.1, folder_path)
            && !desired.contains_key(&current_file.0)
        {
            let had_state = connection
                .query_row(
                    "SELECT 1 FROM file_states WHERE file_id=?1 AND recorded_at<=?2 LIMIT 1",
                    params![current_file.0, target_at],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if current_file.4.as_str() <= target_at && !had_state {
                unavailable.push(format!(
                    "{}: kein vollständiger historischer Pfad",
                    current_file.1
                ));
            } else {
                actions.push(FolderRestoreAction {
                    file_id: current_file.0.clone(),
                    path: current_file.1.clone(),
                    action: "delete".into(),
                });
            }
        }
    }
    actions.sort_by(|left, right| left.path.cmp(&right.path));
    unavailable.sort();
    unavailable.dedup();
    Ok(FolderRestorePreview {
        space_id: space_id.into(),
        folder_path: folder_path.into(),
        target_at: target_at.into(),
        actions,
        unavailable,
    })
}

fn path_is_inside(path: &str, folder_path: &str) -> bool {
    folder_path.is_empty() || path.starts_with(&format!("{folder_path}/"))
}

fn validate_credentials(username: &str, password: &str) -> Result<(), DbError> {
    if username.trim().len() < 3 || username.trim().len() > 80 {
        return Err(DbError::Invalid(
            "Benutzername muss 3 bis 80 Zeichen lang sein".into(),
        ));
    }
    if password.chars().count() < 12 {
        return Err(DbError::Invalid(
            "Passwort muss mindestens 12 Zeichen lang sein".into(),
        ));
    }
    Ok(())
}

fn verify_account_password(
    connection: &Connection,
    user_id: &str,
    password: &str,
) -> Result<(), DbError> {
    let hash: String = connection
        .query_row(
            "SELECT password_hash FROM users WHERE id=?1 AND disabled_at IS NULL",
            [user_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(DbError::NotFound)?;
    let parsed = PasswordHash::new(&hash).map_err(|_| DbError::InvalidLogin)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| DbError::InvalidLogin)
}

fn encrypt_secret(
    password: &str,
    user_id: &str,
    secret: &[u8],
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), DbError> {
    let salt = rand::random::<[u8; 16]>();
    let nonce = rand::random::<[u8; 12]>();
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|error| DbError::Internal(anyhow::anyhow!(error.to_string())))?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&key)
        .map_err(|error| DbError::Internal(anyhow::anyhow!(error.to_string())))?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: user_id.as_bytes(),
            },
        )
        .map_err(|_| DbError::Internal(anyhow::anyhow!("2FA-Verschlüsselung fehlgeschlagen")))?;
    key.fill(0);
    Ok((encrypted, salt.to_vec(), nonce.to_vec()))
}

fn decrypt_secret(
    password: &str,
    user_id: &str,
    cipher_text: &[u8],
    salt: &[u8],
    nonce: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        salt.len() == 16 && nonce.len() == 12,
        "ungültige 2FA-Verschlüsselungsdaten"
    );
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let cipher = <Aes256Gcm as KeyInit>::new_from_slice(&key)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: cipher_text,
                aad: user_id.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("2FA-Secret konnte nicht entschlüsselt werden"))?;
    key.fill(0);
    Ok(plaintext)
}

fn load_encrypted_secret(
    connection: &Connection,
    user_id: &str,
    pending: bool,
) -> Result<Option<(Vec<u8>, Vec<u8>, Vec<u8>)>, DbError> {
    let columns = if pending {
        "pending_totp_cipher,pending_totp_salt,pending_totp_nonce"
    } else {
        "totp_cipher,totp_salt,totp_nonce"
    };
    let values = connection
        .query_row(
            &format!("SELECT {columns} FROM users WHERE id=?1"),
            [user_id],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(DbError::NotFound)?;
    match values {
        (None, None, None) => Ok(None),
        (Some(cipher), Some(salt), Some(nonce)) => Ok(Some((cipher, salt, nonce))),
        _ => Err(DbError::Internal(anyhow::anyhow!(
            "unvollständige 2FA-Daten"
        ))),
    }
}

fn rekey_two_factor(
    connection: &Connection,
    user_id: &str,
    old_password: &str,
    new_password: &str,
) -> Result<(), DbError> {
    for pending in [false, true] {
        let Some((cipher, salt, nonce)) = load_encrypted_secret(connection, user_id, pending)?
        else {
            continue;
        };
        let secret = decrypt_secret(old_password, user_id, &cipher, &salt, &nonce)
            .map_err(DbError::Internal)?;
        let (cipher, salt, nonce) = encrypt_secret(new_password, user_id, &secret)?;
        let columns = if pending {
            (
                "pending_totp_cipher",
                "pending_totp_salt",
                "pending_totp_nonce",
            )
        } else {
            ("totp_cipher", "totp_salt", "totp_nonce")
        };
        connection.execute(
            &format!(
                "UPDATE users SET {}=?1,{}=?2,{}=?3 WHERE id=?4",
                columns.0, columns.1, columns.2
            ),
            params![cipher, salt, nonce, user_id],
        )?;
    }
    Ok(())
}

fn verify_totp(secret: &[u8], code: &str, unix_time: i64) -> bool {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    (-1_i64..=1).any(|offset| totp_code(secret, unix_time.div_euclid(30) + offset) == code)
}

fn totp_code(secret: &[u8], counter: i64) -> String {
    type HmacSha1 = Hmac<Sha1>;
    let mut mac = <HmacSha1 as Mac>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&(counter as u64).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    format!("{:06}", binary % 1_000_000)
}

fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

fn validate_quota(quota: Option<i64>) -> Result<(), DbError> {
    if quota.is_some_and(|value| value < 1024 * 1024) {
        return Err(DbError::Invalid(
            "Quote muss mindestens 1 MiB betragen oder leer sein".into(),
        ));
    }
    Ok(())
}

fn enforce_quotas(
    transaction: &Transaction<'_>,
    space_id: &str,
    additional_bytes: i64,
) -> Result<(), DbError> {
    let (owner_id, space_quota): (String, Option<i64>) = transaction.query_row(
        "SELECT owner_user_id,quota_bytes FROM spaces WHERE id=?1 AND deleted_at IS NULL",
        [space_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let space_usage: i64 = transaction.query_row(
        "SELECT COALESCE(SUM(v.size),0) FROM versions v JOIN files f ON f.id=v.file_id WHERE f.space_id=?1",
        [space_id],
        |row| row.get(0),
    )?;
    if space_quota.is_some_and(|quota| space_usage.saturating_add(additional_bytes) > quota) {
        return Err(DbError::Conflict(
            "Space-Quote ist ausgeschöpft; die letzte bestätigte Version bleibt unverändert".into(),
        ));
    }
    let user_quota: Option<i64> = transaction.query_row(
        "SELECT quota_bytes FROM users WHERE id=?1",
        [&owner_id],
        |row| row.get(0),
    )?;
    if let Some(quota) = user_quota {
        let user_usage: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(v.size),0) FROM versions v JOIN files f ON f.id=v.file_id
             JOIN spaces s ON s.id=f.space_id WHERE s.owner_user_id=?1",
            [&owner_id],
            |row| row.get(0),
        )?;
        if user_usage.saturating_add(additional_bytes) > quota {
            return Err(DbError::Conflict(
                "Nutzerquote ist ausgeschöpft; die letzte bestätigte Version bleibt unverändert"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn migrate_schema(connection: &mut Connection) -> anyhow::Result<()> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS directories (
           id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES spaces(id), path TEXT NOT NULL,
           created_at TEXT NOT NULL, deleted_at TEXT
         );
         CREATE UNIQUE INDEX IF NOT EXISTS directories_live_path ON directories(space_id,path) WHERE deleted_at IS NULL;",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS backup_runs (
           id TEXT PRIMARY KEY, destination TEXT NOT NULL, started_at TEXT NOT NULL, completed_at TEXT,
           status TEXT NOT NULL CHECK(status IN ('running','completed','failed')), entries INTEGER, bytes INTEGER, error TEXT
         );",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS file_states (
           id INTEGER PRIMARY KEY AUTOINCREMENT, file_id TEXT NOT NULL, space_id TEXT NOT NULL,
           path TEXT NOT NULL, version_id TEXT, deleted INTEGER NOT NULL, recorded_at TEXT NOT NULL,
           change_sequence INTEGER UNIQUE
         );
         CREATE INDEX IF NOT EXISTS file_states_lookup ON file_states(space_id,file_id,id);",
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS server_settings (
           key TEXT PRIMARY KEY, value TEXT NOT NULL
         );",
    )?;
    if table_exists(&transaction, "files")? && table_exists(&transaction, "changes")? {
        transaction.execute_batch(
            "INSERT INTO file_states (file_id,space_id,path,version_id,deleted,recorded_at,change_sequence)
             SELECT f.id,f.space_id,f.path,f.current_version_id,f.deleted_at IS NOT NULL,f.updated_at,
                    (SELECT MAX(c.sequence) FROM changes c WHERE c.object_id=f.id)
             FROM files f WHERE NOT EXISTS (SELECT 1 FROM file_states fs WHERE fs.file_id=f.id);
             CREATE TRIGGER IF NOT EXISTS record_file_state_after_change AFTER INSERT ON changes
             WHEN EXISTS (SELECT 1 FROM files f WHERE f.id=NEW.object_id)
             BEGIN
               INSERT OR IGNORE INTO file_states (file_id,space_id,path,version_id,deleted,recorded_at,change_sequence)
               SELECT f.id,f.space_id,f.path,f.current_version_id,f.deleted_at IS NOT NULL,NEW.created_at,NEW.sequence
               FROM files f WHERE f.id=NEW.object_id;
             END;",
        )?;
    }
    if !column_exists(&transaction, "users", "quota_bytes")? {
        transaction.execute("ALTER TABLE users ADD COLUMN quota_bytes INTEGER", [])?;
    }
    if !column_exists(&transaction, "spaces", "quota_bytes")? {
        transaction.execute("ALTER TABLE spaces ADD COLUMN quota_bytes INTEGER", [])?;
    }
    if !column_exists(&transaction, "spaces", "archive_protected")? {
        transaction.execute(
            "ALTER TABLE spaces ADD COLUMN archive_protected INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(&transaction, "spaces", "editor_sharing")? {
        transaction.execute(
            "ALTER TABLE spaces ADD COLUMN editor_sharing INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    for column in [
        "totp_cipher",
        "totp_salt",
        "totp_nonce",
        "pending_totp_cipher",
        "pending_totp_salt",
        "pending_totp_nonce",
    ] {
        if !column_exists(&transaction, "users", column)? {
            transaction.execute(&format!("ALTER TABLE users ADD COLUMN {column} BLOB"), [])?;
        }
    }
    let now = Utc::now().to_rfc3339();
    let current_version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if current_version < 12
        && table_exists(&transaction, "files")?
        && table_exists(&transaction, "uploads")?
    {
        transaction.execute_batch(
            "UPDATE uploads SET status='aborted'
             WHERE status='open' AND space_id IN (SELECT id FROM spaces WHERE deleted_at IS NOT NULL);",
        )?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (1,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (2,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (3,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (4,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (5,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (6,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (7,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (8,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (9,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (10,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (11,?1)",
        [&now],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version,applied_at) VALUES (12,?1)",
        [&now],
    )?;
    transaction.commit()?;
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_exists(connection: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn hash_password(password: &str) -> Result<String, DbError> {
    let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
        .map_err(|error| DbError::Internal(anyhow::anyhow!(error.to_string())))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| DbError::Internal(anyhow::anyhow!(error.to_string())))?
        .to_string())
}

fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn share_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShareRow> {
    Ok(ShareRow {
        id: row.get(0)?,
        space_id: row.get(1)?,
        target_path: row.get(2)?,
        file_id: row.get(3)?,
        mode: row.get(4)?,
        password_protected: row.get(5)?,
        expires_at: row.get(6)?,
        revoked_at: row.get(7)?,
        max_upload_size: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn create_session(
    transaction: &Transaction<'_>,
    user_id: &str,
    username: &str,
    is_admin: bool,
    device_name: &str,
    personal_space_id: &str,
) -> Result<SessionResult, DbError> {
    let device_id = Uuid::now_v7().to_string();
    let token = format!("cs_{}", hex::encode(rand::random::<[u8; 32]>()));
    let now = Utc::now();
    transaction.execute(
        "INSERT INTO devices (id,user_id,name,created_at,last_seen_at) VALUES (?1,?2,?3,?4,?4)",
        params![device_id, user_id, device_name.trim(), now.to_rfc3339()],
    )?;
    transaction.execute(
        "INSERT INTO sessions (id,device_id,token_hash,created_at,expires_at) VALUES (?1,?2,?3,?4,?5)",
        params![Uuid::now_v7().to_string(),device_id,hash_token(&token),now.to_rfc3339(),(now+Duration::days(90)).to_rfc3339()],
    )?;
    Ok(SessionResult {
        token,
        user_id: user_id.into(),
        username: username.into(),
        device_id,
        personal_space_id: personal_space_id.into(),
        is_admin,
    })
}

fn require_permission_connection(
    connection: &Connection,
    space_id: &str,
    user_id: &str,
    permission: Permission,
) -> Result<String, DbError> {
    let mut statement = connection.prepare(
        "SELECT m.role FROM memberships m JOIN spaces s ON s.id=m.space_id
         WHERE m.space_id=?1 AND m.user_id=?2 AND s.deleted_at IS NULL
         UNION ALL
         SELECT gsr.role FROM group_space_roles gsr JOIN group_members gm ON gm.group_id=gsr.group_id
         JOIN spaces s ON s.id=gsr.space_id
         WHERE gsr.space_id=?1 AND gm.user_id=?2 AND s.deleted_at IS NULL",
    )?;
    let roles = statement
        .query_map(params![space_id, user_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let allowed: Vec<_> = roles
        .into_iter()
        .filter(|role| role_allows(role, permission))
        .collect();
    if allowed.is_empty() {
        return Err(DbError::Forbidden);
    }
    Ok(allowed
        .into_iter()
        .max_by_key(|role| role_rank(role))
        .expect("not empty"))
}

fn require_share_permission(
    connection: &Connection,
    space_id: &str,
    user_id: &str,
) -> Result<(), DbError> {
    let role = require_permission_connection(connection, space_id, user_id, Permission::Edit)?;
    if role == "editor" {
        let enabled: bool = connection
            .query_row(
                "SELECT editor_sharing FROM spaces WHERE id=?1 AND deleted_at IS NULL",
                [space_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        if !enabled {
            return Err(DbError::Forbidden);
        }
    }
    Ok(())
}

fn role_rank(role: &str) -> i32 {
    match role {
        "uploader" => 0,
        "viewer" => 1,
        "editor" => 2,
        "manager" => 3,
        "owner" => 4,
        _ => -1,
    }
}

fn role_allows(role: &str, permission: Permission) -> bool {
    match permission {
        Permission::View => matches!(role, "viewer" | "editor" | "manager" | "owner"),
        Permission::Upload => matches!(role, "uploader" | "editor" | "manager" | "owner"),
        Permission::Edit => matches!(role, "editor" | "manager" | "owner"),
        Permission::Manage => matches!(role, "manager" | "owner"),
        Permission::Own => role == "owner",
    }
}

fn upload_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UploadRow> {
    let json: String = row.get(8)?;
    let blocks = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(UploadRow {
        id: row.get(0)?,
        space_id: row.get(1)?,
        user_id: row.get(2)?,
        device_id: row.get(3)?,
        path: row.get(4)?,
        file_id: row.get(5)?,
        base_version_id: row.get(6)?,
        size: row.get(7)?,
        blocks,
        idempotency_key: row.get(9)?,
        expires_at: row.get(10)?,
        status: row.get(11)?,
    })
}

const UPLOAD_SELECT: &str = "SELECT id,space_id,user_id,device_id,path,file_id,base_version_id,size,blocks_json,idempotency_key,expires_at,status FROM uploads";

fn commit_upload_on(
    transaction: &Transaction<'_>,
    upload_id: &str,
    auth: &AuthContext,
) -> Result<CommitResult, DbError> {
    let upload = load_upload(transaction, upload_id)?.ok_or(DbError::NotFound)?;
    if upload.user_id != auth.user_id || upload.device_id != auth.device_id {
        return Err(DbError::Forbidden);
    }
    if upload.status == "committed" {
        let json: String = transaction.query_row(
            "SELECT response_json FROM idempotency_results WHERE device_id=?1 AND idempotency_key=?2",
            params![auth.device_id, upload.idempotency_key],
            |row| row.get(0),
        )?;
        return Ok(serde_json::from_str(&json)?);
    }
    if upload.status != "open" || upload.expires_at <= Utc::now().to_rfc3339() {
        return Err(DbError::Conflict(
            "Upload ist abgelaufen oder geschlossen".into(),
        ));
    }
    let role = require_permission_connection(
        transaction,
        &upload.space_id,
        &auth.user_id,
        Permission::Upload,
    )?;
    enforce_quotas(transaction, &upload.space_id, upload.size)?;

    let now = Utc::now();
    let existing = find_target_file(transaction, &upload)?;
    let (file_id, final_path, conflict) = match existing {
        Some((id, current_version, existing_path)) => {
            if role == "uploader" {
                (
                    Uuid::now_v7().to_string(),
                    available_upload_path(transaction, &upload.space_id, &upload.path)?,
                    true,
                )
            } else {
                let same_base = upload.base_version_id.as_deref() == current_version.as_deref();
                let targets_same_file = upload
                    .file_id
                    .as_deref()
                    .is_none_or(|candidate| candidate == id);
                if same_base && targets_same_file {
                    (id, existing_path, false)
                } else {
                    (
                        Uuid::now_v7().to_string(),
                        conflict_path(&upload.path, &auth.device_name, now),
                        true,
                    )
                }
            }
        }
        None if upload.file_id.is_some() => (
            Uuid::now_v7().to_string(),
            conflict_path(&upload.path, &auth.device_name, now),
            true,
        ),
        None => (Uuid::now_v7().to_string(), upload.path.clone(), false),
    };

    let now_text = now.to_rfc3339();
    ensure_file_target(transaction, &upload.space_id, &final_path)?;
    ensure_parent_directories(transaction, &upload.space_id, &final_path, &now_text)?;
    if transaction
        .query_row("SELECT 1 FROM files WHERE id=?1", [&file_id], |_| Ok(()))
        .optional()?
        .is_none()
    {
        transaction.execute(
            "INSERT INTO files (id,space_id,path,size,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?5)",
            params![file_id, upload.space_id, final_path, upload.size, now_text],
        )?;
    }
    let version_id = Uuid::now_v7().to_string();
    transaction.execute(
        "INSERT INTO versions (id,file_id,size,created_by_user_id,created_by_device_id,created_at,format)
         VALUES (?1,?2,?3,?4,?5,?6,1)",
        params![version_id,file_id,upload.size,auth.user_id,auth.device_id,now_text],
    )?;
    for (position, block) in upload.blocks.iter().enumerate() {
        transaction.execute(
            "INSERT INTO blocks (space_id,hash,size,reference_count,created_at) VALUES (?1,?2,?3,1,?4)
             ON CONFLICT(space_id,hash) DO UPDATE SET reference_count=reference_count+1",
            params![upload.space_id, block.hash, block.size, now_text],
        )?;
        transaction.execute(
            "INSERT INTO version_blocks (version_id,position,block_hash,block_size) VALUES (?1,?2,?3,?4)",
            params![version_id, position as i64, block.hash, block.size],
        )?;
    }
    transaction.execute(
        "UPDATE files SET path=?1,size=?2,current_version_id=?3,updated_at=?4,deleted_at=NULL WHERE id=?5",
        params![final_path,upload.size,version_id,now_text,file_id],
    )?;
    let event_id = Uuid::now_v7().to_string();
    transaction.execute(
        "INSERT INTO changes (event_id,space_id,object_id,kind,path,version_id,user_id,device_id,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![event_id,upload.space_id,file_id,if conflict {"conflict_created"} else {"content_changed"},final_path,version_id,auth.user_id,auth.device_id,now_text],
    )?;
    let sequence = transaction.last_insert_rowid();
    let result = CommitResult {
        file_id,
        version_id,
        path: final_path,
        conflict,
        change_sequence: sequence,
    };
    transaction.execute(
        "UPDATE uploads SET status='committed' WHERE id=?1",
        [&upload.id],
    )?;
    transaction.execute(
        "INSERT INTO idempotency_results (device_id,idempotency_key,response_json,created_at) VALUES (?1,?2,?3,?4)",
        params![auth.device_id,upload.idempotency_key,serde_json::to_string(&result)?,now_text],
    )?;
    Ok(result)
}

fn create_upload_on(
    connection: &Connection,
    auth: &AuthContext,
    space_id: &str,
    input: &NewUpload,
) -> Result<UploadRow, DbError> {
    let role =
        require_permission_connection(connection, space_id, &auth.user_id, Permission::Upload)?;
    if role == "uploader" && (input.file_id.is_some() || input.base_version_id.is_some()) {
        return Err(DbError::Forbidden);
    }
    if input.idempotency_key.len() < 8 || input.idempotency_key.len() > 200 {
        return Err(DbError::Invalid(
            "Idempotenzschlüssel muss 8 bis 200 Zeichen lang sein".into(),
        ));
    }
    let total: i64 = input.blocks.iter().map(|block| i64::from(block.size)).sum();
    if total != input.size
        || input.blocks.iter().any(|block| {
            block.hash.len() != 64 || !block.hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(DbError::Invalid(
            "Blockliste stimmt nicht mit der Dateigröße überein".into(),
        ));
    }
    if let Some(existing) = load_upload_by_key(connection, &auth.device_id, &input.idempotency_key)?
    {
        return Ok(existing);
    }
    let upload = UploadRow {
        id: Uuid::now_v7().to_string(),
        space_id: space_id.into(),
        user_id: auth.user_id.clone(),
        device_id: auth.device_id.clone(),
        path: input.path.clone(),
        file_id: input.file_id.clone(),
        base_version_id: input.base_version_id.clone(),
        size: input.size,
        blocks: input.blocks.clone(),
        idempotency_key: input.idempotency_key.clone(),
        expires_at: (Utc::now() + Duration::hours(24)).to_rfc3339(),
        status: "open".into(),
    };
    connection.execute(
        "INSERT INTO uploads (id,space_id,user_id,device_id,path,file_id,base_version_id,size,blocks_json,idempotency_key,expires_at,status,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'open',?12)",
        params![upload.id, upload.space_id, upload.user_id, upload.device_id, upload.path,
            upload.file_id, upload.base_version_id, upload.size, serde_json::to_string(&upload.blocks)?,
            upload.idempotency_key, upload.expires_at, Utc::now().to_rfc3339()],
    )?;
    Ok(upload)
}

fn load_upload(connection: &Connection, id: &str) -> Result<Option<UploadRow>, DbError> {
    Ok(connection
        .query_row(
            &format!("{UPLOAD_SELECT} WHERE id=?1"),
            [id],
            upload_from_row,
        )
        .optional()?)
}

fn load_upload_by_key(
    connection: &Connection,
    device_id: &str,
    key: &str,
) -> Result<Option<UploadRow>, DbError> {
    Ok(connection
        .query_row(
            &format!("{UPLOAD_SELECT} WHERE device_id=?1 AND idempotency_key=?2"),
            params![device_id, key],
            upload_from_row,
        )
        .optional()?)
}

fn find_target_file(
    transaction: &Transaction<'_>,
    upload: &UploadRow,
) -> Result<Option<(String, Option<String>, String)>, DbError> {
    let result = if let Some(file_id) = &upload.file_id {
        transaction.query_row("SELECT id,current_version_id,path FROM files WHERE id=?1 AND space_id=?2 AND deleted_at IS NULL",
            params![file_id,upload.space_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?
    } else {
        transaction.query_row("SELECT id,current_version_id,path FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
            params![upload.space_id,upload.path], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?
    };
    Ok(result)
}

fn conflict_path(path: &str, device_name: &str, now: DateTime<Utc>) -> String {
    let (stem, extension) = path
        .rsplit_once('.')
        .map_or((path, ""), |(left, right)| (left, right));
    let safe_device: String = device_name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let suffix = format!(
        " (Konflikt {} {})",
        safe_device,
        now.format("%Y-%m-%d %H%M%S UTC")
    );
    if extension.is_empty() {
        format!("{stem}{suffix}")
    } else {
        format!("{stem}{suffix}.{extension}")
    }
}

fn insert_change(
    transaction: &Transaction<'_>,
    space_id: &str,
    file_id: &str,
    kind: &str,
    path: &str,
    version_id: Option<&str>,
    auth: &AuthContext,
    now: &str,
) -> Result<i64, DbError> {
    transaction.execute(
        "INSERT INTO changes (event_id,space_id,object_id,kind,path,version_id,user_id,device_id,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![Uuid::now_v7().to_string(),space_id,file_id,kind,path,version_id,auth.user_id,auth.device_id,now],
    )?;
    Ok(transaction.last_insert_rowid())
}

fn ensure_parent_directories(
    transaction: &Transaction<'_>,
    space_id: &str,
    file_path: &str,
    now: &str,
) -> Result<(), DbError> {
    let parts: Vec<_> = file_path.split('/').collect();
    for end in 1..parts.len() {
        let path = parts[..end].join("/");
        transaction.execute(
            "INSERT OR IGNORE INTO directories (id,space_id,path,created_at) VALUES (?1,?2,?3,?4)",
            params![Uuid::now_v7().to_string(), space_id, path, now],
        )?;
    }
    Ok(())
}

fn file_ancestor_exists(
    transaction: &Transaction<'_>,
    space_id: &str,
    path: &str,
) -> Result<bool, DbError> {
    let parts: Vec<_> = path.split('/').collect();
    for end in 1..parts.len() {
        let ancestor = parts[..end].join("/");
        if transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, ancestor],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_file_target(
    transaction: &Transaction<'_>,
    space_id: &str,
    path: &str,
) -> Result<(), DbError> {
    if file_ancestor_exists(transaction, space_id, path)? {
        return Err(DbError::Conflict(
            "Eine Datei im Zielpfad verhindert diese Datei".into(),
        ));
    }
    let directory_prefix = format!("{}/%", escape_like(path));
    let directory_conflict = transaction
        .query_row(
            "SELECT 1 FROM directories WHERE space_id=?1 AND deleted_at IS NULL AND (path=?2 OR path LIKE ?3 ESCAPE '\\') LIMIT 1",
            params![space_id, path, directory_prefix],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if directory_conflict {
        return Err(DbError::Conflict(
            "Am Ziel existiert bereits ein Ordner".into(),
        ));
    }
    Ok(())
}

fn available_restore_path(
    transaction: &Transaction<'_>,
    space_id: &str,
    original: &str,
) -> Result<String, DbError> {
    let (stem, extension) = original
        .rsplit_once('.')
        .map_or((original, ""), |(left, right)| (left, right));
    for number in 1..=10_000 {
        let candidate = if extension.is_empty() {
            format!("{stem} (wiederhergestellt {number})")
        } else {
            format!("{stem} (wiederhergestellt {number}).{extension}")
        };
        let occupied = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, candidate],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !occupied {
            return Ok(candidate);
        }
    }
    Err(DbError::Conflict(
        "Kein freier Wiederherstellungsname gefunden".into(),
    ))
}

fn available_upload_path(
    transaction: &Transaction<'_>,
    space_id: &str,
    original: &str,
) -> Result<String, DbError> {
    let (stem, extension) = original
        .rsplit_once('.')
        .map_or((original, ""), |(left, right)| (left, right));
    for number in 1..=10_000 {
        let candidate = if extension.is_empty() {
            format!("{stem} ({number})")
        } else {
            format!("{stem} ({number}).{extension}")
        };
        let occupied = transaction
            .query_row(
                "SELECT 1 FROM files WHERE space_id=?1 AND path=?2 AND deleted_at IS NULL",
                params![space_id, candidate],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !occupied {
            return Ok(candidate);
        }
    }
    Err(DbError::Conflict("Kein freier Dateiname gefunden".into()))
}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> Self {
        DbError::Internal(error.into())
    }
}
impl From<serde_json::Error> for DbError {
    fn from(error: serde_json::Error) -> Self {
        DbError::Internal(error.into())
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE, password_hash TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, disabled_at TEXT, quota_bytes INTEGER,
  totp_cipher BLOB, totp_salt BLOB, totp_nonce BLOB,
  pending_totp_cipher BLOB, pending_totp_salt BLOB, pending_totp_nonce BLOB
);
CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id), name TEXT NOT NULL,
  created_at TEXT NOT NULL, last_seen_at TEXT NOT NULL, revoked_at TEXT
);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id), token_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL, expires_at TEXT NOT NULL, revoked_at TEXT
);
CREATE TABLE IF NOT EXISTS spaces (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('personal','shared')),
  owner_user_id TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, deleted_at TEXT, quota_bytes INTEGER,
  archive_protected INTEGER NOT NULL DEFAULT 0, editor_sharing INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS memberships (
  space_id TEXT NOT NULL REFERENCES spaces(id), user_id TEXT NOT NULL REFERENCES users(id),
  role TEXT NOT NULL CHECK(role IN ('owner','manager','editor','viewer','uploader')),
  PRIMARY KEY(space_id,user_id)
);
CREATE TABLE IF NOT EXISTS groups (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS group_members (
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE, PRIMARY KEY(group_id,user_id)
);
CREATE TABLE IF NOT EXISTS group_space_roles (
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
  space_id TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('manager','editor','viewer','uploader')),
  PRIMARY KEY(group_id,space_id)
);
CREATE TABLE IF NOT EXISTS files (
  id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES spaces(id), path TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0, current_version_id TEXT, created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL, deleted_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS files_live_path ON files(space_id,path) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS directories (
  id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES spaces(id), path TEXT NOT NULL,
  created_at TEXT NOT NULL, deleted_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS directories_live_path ON directories(space_id,path) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS versions (
  id TEXT PRIMARY KEY, file_id TEXT NOT NULL REFERENCES files(id), size INTEGER NOT NULL,
  created_by_user_id TEXT NOT NULL REFERENCES users(id), created_by_device_id TEXT NOT NULL REFERENCES devices(id),
  created_at TEXT NOT NULL, format INTEGER NOT NULL, pinned_name TEXT
);
CREATE TABLE IF NOT EXISTS version_blocks (
  version_id TEXT NOT NULL REFERENCES versions(id), position INTEGER NOT NULL,
  block_hash TEXT NOT NULL, block_size INTEGER NOT NULL, PRIMARY KEY(version_id,position)
);
CREATE TABLE IF NOT EXISTS blocks (
  space_id TEXT NOT NULL REFERENCES spaces(id), hash TEXT NOT NULL, size INTEGER NOT NULL,
  reference_count INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, PRIMARY KEY(space_id,hash)
);
CREATE TABLE IF NOT EXISTS uploads (
  id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES spaces(id), user_id TEXT NOT NULL REFERENCES users(id),
  device_id TEXT NOT NULL REFERENCES devices(id), path TEXT NOT NULL, file_id TEXT, base_version_id TEXT,
  size INTEGER NOT NULL, blocks_json TEXT NOT NULL, idempotency_key TEXT NOT NULL, expires_at TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('open','committed','aborted')), created_at TEXT NOT NULL,
  UNIQUE(device_id,idempotency_key)
);
CREATE TABLE IF NOT EXISTS idempotency_results (
  device_id TEXT NOT NULL REFERENCES devices(id), idempotency_key TEXT NOT NULL,
  response_json TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY(device_id,idempotency_key)
);
CREATE TABLE IF NOT EXISTS webhooks (
  id TEXT PRIMARY KEY, space_id TEXT NOT NULL REFERENCES spaces(id), url TEXT NOT NULL,
  signing_secret TEXT NOT NULL, created_by_user_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL, disabled_at TEXT
);
CREATE TABLE IF NOT EXISTS changes (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE, space_id TEXT NOT NULL REFERENCES spaces(id),
  object_id TEXT NOT NULL, kind TEXT NOT NULL, path TEXT NOT NULL, version_id TEXT,
  user_id TEXT NOT NULL REFERENCES users(id), device_id TEXT NOT NULL REFERENCES devices(id), created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS webhook_deliveries (
  id TEXT PRIMARY KEY, webhook_id TEXT NOT NULL REFERENCES webhooks(id),
  event_id TEXT NOT NULL REFERENCES changes(event_id), status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at TEXT NOT NULL, delivered_at TEXT, last_error TEXT,
  UNIQUE(webhook_id,event_id)
);
CREATE TRIGGER IF NOT EXISTS enqueue_webhooks_after_change AFTER INSERT ON changes
BEGIN
  INSERT INTO webhook_deliveries (id,webhook_id,event_id,next_attempt_at)
  SELECT w.id || ':' || NEW.event_id,w.id,NEW.event_id,NEW.created_at
  FROM webhooks w WHERE w.space_id=NEW.space_id AND w.disabled_at IS NULL;
END;
CREATE TABLE IF NOT EXISTS shares (
  id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, space_id TEXT NOT NULL REFERENCES spaces(id),
  target_path TEXT NOT NULL, file_id TEXT REFERENCES files(id), mode TEXT NOT NULL CHECK(mode IN ('read','upload')),
  password_hash TEXT, expires_at TEXT, revoked_at TEXT, max_upload_size INTEGER,
  created_by_user_id TEXT NOT NULL REFERENCES users(id), guest_device_id TEXT NOT NULL REFERENCES devices(id),
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS recovery_codes (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id), code_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL, used_at TEXT
);
CREATE TABLE IF NOT EXISTS backup_runs (
  id TEXT PRIMARY KEY, destination TEXT NOT NULL, started_at TEXT NOT NULL, completed_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('running','completed','failed')), entries INTEGER, bytes INTEGER, error TEXT
);
CREATE TABLE IF NOT EXISTS file_states (
  id INTEGER PRIMARY KEY AUTOINCREMENT, file_id TEXT NOT NULL, space_id TEXT NOT NULL,
  path TEXT NOT NULL, version_id TEXT, deleted INTEGER NOT NULL, recorded_at TEXT NOT NULL,
  change_sequence INTEGER UNIQUE
);
CREATE INDEX IF NOT EXISTS file_states_lookup ON file_states(space_id,file_id,id);
CREATE TABLE IF NOT EXISTS server_settings (
  key TEXT PRIMARY KEY, value TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS record_file_state_after_change AFTER INSERT ON changes
WHEN EXISTS (SELECT 1 FROM files f WHERE f.id=NEW.object_id)
BEGIN
  INSERT OR IGNORE INTO file_states (file_id,space_id,path,version_id,deleted,recorded_at,change_sequence)
  SELECT f.id,f.space_id,f.path,f.current_version_id,f.deleted_at IS NOT NULL,NEW.created_at,NEW.sequence
  FROM files f WHERE f.id=NEW.object_id;
END;
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL
);
"#;

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn adds_quota_columns_to_legacy_schema_idempotently() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE users (id TEXT PRIMARY KEY);
             CREATE TABLE spaces (id TEXT PRIMARY KEY);
             CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        ).unwrap();
        migrate_schema(&mut connection).unwrap();
        migrate_schema(&mut connection).unwrap();
        assert!(column_exists(&connection, "users", "quota_bytes").unwrap());
        assert!(column_exists(&connection, "spaces", "quota_bytes").unwrap());
        assert!(column_exists(&connection, "spaces", "archive_protected").unwrap());
        assert!(column_exists(&connection, "spaces", "editor_sharing").unwrap());
        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 12);
    }

    #[test]
    fn upgrades_existing_v6_data_and_preserves_sessions_files_and_history_seed() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("metadata.db");
        let database = Database::open(&path).unwrap();
        let session = database
            .setup_admin("admin", "a very long secure password", "Altgerät")
            .unwrap();
        let auth = database.authenticate(&session.token).unwrap();
        let upload = database
            .create_upload(
                &auth,
                &session.personal_space_id,
                "Bestand/bleibt.txt",
                None,
                None,
                0,
                &[],
                "migration-existing-01",
            )
            .unwrap();
        let committed = database.commit_upload(&upload.id, &auth).unwrap();
        {
            let connection = database.lock().unwrap();
            connection
                .execute_batch(
                    "DROP TRIGGER IF EXISTS record_file_state_after_change;
                     DROP TABLE file_states;
                     DROP TABLE backup_runs;
                     DROP TABLE directories;
                     DELETE FROM schema_migrations WHERE version>6;",
                )
                .unwrap();
        }
        drop(database);

        let upgraded = Database::open(&path).unwrap();
        assert_eq!(upgraded.schema_version().unwrap(), 12);
        let restored_auth = upgraded.authenticate(&session.token).unwrap();
        let files = upgraded
            .list_files(&session.personal_space_id, &restored_auth.user_id, false)
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, committed.file_id);
        assert_eq!(files[0].path, "Bestand/bleibt.txt");
        let states: i64 = upgraded
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM file_states WHERE file_id=?1",
                [&committed.file_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(states, 1);
    }

    #[test]
    fn retention_keeps_current_and_one_version_per_hour_bucket() {
        let database = Database::open(":memory:").unwrap();
        let session = database
            .setup_admin("admin", "a very long secure password", "Test")
            .unwrap();
        let auth = database.authenticate(&session.token).unwrap();
        let first = database
            .create_upload(
                &auth,
                &session.personal_space_id,
                "history.txt",
                None,
                None,
                0,
                &[],
                "retention-0001",
            )
            .and_then(|upload| database.commit_upload(&upload.id, &auth))
            .unwrap();
        let second = database
            .create_upload(
                &auth,
                &session.personal_space_id,
                "history.txt",
                Some(&first.file_id),
                Some(&first.version_id),
                0,
                &[],
                "retention-0002",
            )
            .and_then(|upload| database.commit_upload(&upload.id, &auth))
            .unwrap();
        let third = database
            .create_upload(
                &auth,
                &session.personal_space_id,
                "history.txt",
                Some(&first.file_id),
                Some(&second.version_id),
                0,
                &[],
                "retention-0003",
            )
            .and_then(|upload| database.commit_upload(&upload.id, &auth))
            .unwrap();
        let now = Utc::now();
        {
            let connection = database.lock().unwrap();
            connection
                .execute(
                    "UPDATE versions SET created_at=?1 WHERE id=?2",
                    params![(now - Duration::days(3)).to_rfc3339(), first.version_id],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE versions SET created_at=?1 WHERE id=?2",
                    params![(now - Duration::days(3)).to_rfc3339(), second.version_id],
                )
                .unwrap();
        }
        let result = database.apply_default_retention(now).unwrap();
        assert_eq!(result.deleted_versions, 1);
        let versions = database
            .list_versions(&first.file_id, &auth.user_id)
            .unwrap();
        assert_eq!(versions.len(), 2);
        assert!(
            versions
                .iter()
                .any(|version| version.id == third.version_id && version.current)
        );
    }

    #[test]
    fn deleting_space_hides_current_data_aborts_uploads_and_allows_retention() {
        let database = Database::open(":memory:").unwrap();
        let session = database
            .setup_admin("admin", "a very long secure password", "Test")
            .unwrap();
        let auth = database.authenticate(&session.token).unwrap();
        let space = database.create_space(&auth.user_id, "Temporär").unwrap();
        let committed = database
            .create_upload(
                &auth,
                &space.id,
                "fertig.txt",
                None,
                None,
                0,
                &[],
                "delete-space-committed",
            )
            .and_then(|upload| database.commit_upload(&upload.id, &auth))
            .unwrap();
        let pending = database
            .create_upload(
                &auth,
                &space.id,
                "angefangen.txt",
                None,
                None,
                0,
                &[],
                "delete-space-pending",
            )
            .unwrap();

        let before = database.system_status(&auth).unwrap();
        assert_eq!(before.spaces, 2);
        assert_eq!(before.current_files, 1);
        assert_eq!(before.open_uploads, 1);

        database.delete_space(&auth, &space.id, "Temporär").unwrap();

        let after = database.system_status(&auth).unwrap();
        assert_eq!(after.spaces, 1);
        assert_eq!(after.current_files, 0);
        assert_eq!(after.trashed_files, 0);
        assert_eq!(after.current_bytes, 0);
        assert_eq!(after.open_uploads, 0);
        {
            let connection = database.lock().unwrap();
            let file_deleted: bool = connection
                .query_row(
                    "SELECT deleted_at IS NOT NULL FROM files WHERE id=?1",
                    [&committed.file_id],
                    |row| row.get(0),
                )
                .unwrap();
            let upload_status: String = connection
                .query_row(
                    "SELECT status FROM uploads WHERE id=?1",
                    [&pending.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!file_deleted);
            assert_eq!(upload_status, "aborted");
            connection
                .execute(
                    "UPDATE spaces SET deleted_at=?1 WHERE id=?2",
                    params![(Utc::now() - Duration::days(31)).to_rfc3339(), space.id],
                )
                .unwrap();
        }
        let retention = database.apply_default_retention(Utc::now()).unwrap();
        assert_eq!(retention.purged_files, 1);
    }

    #[test]
    fn two_factor_is_required_and_survives_password_rekey() {
        assert_eq!(totp_code(b"12345678901234567890", 1), "287082");
        let database = Database::open(":memory:").unwrap();
        let session = database
            .setup_admin("admin", "a very long secure password", "Test")
            .unwrap();
        let auth = database.authenticate(&session.token).unwrap();
        database
            .begin_two_factor(&auth, "a very long secure password")
            .unwrap();
        let secret = {
            let connection = database.lock().unwrap();
            let encrypted = load_encrypted_secret(&connection, &auth.user_id, true)
                .unwrap()
                .unwrap();
            decrypt_secret(
                "a very long secure password",
                &auth.user_id,
                &encrypted.0,
                &encrypted.1,
                &encrypted.2,
            )
            .unwrap()
        };
        let code = totp_code(&secret, Utc::now().timestamp().div_euclid(30));
        database
            .confirm_two_factor(&auth, "a very long secure password", &code)
            .unwrap();
        assert!(matches!(
            database.login("admin", "a very long secure password", "No OTP"),
            Err(DbError::InvalidLogin)
        ));
        database
            .login_with_otp("admin", "a very long secure password", "OTP", Some(&code))
            .unwrap();
        database
            .change_password(
                &auth,
                "a very long secure password",
                "a different secure password",
                &session.token,
            )
            .unwrap();
        database
            .login_with_otp("admin", "a different secure password", "Rekey", Some(&code))
            .unwrap();
        database
            .disable_two_factor(&auth, "a different secure password", &code)
            .unwrap();
        database
            .login("admin", "a different secure password", "No OTP again")
            .unwrap();
    }
}
