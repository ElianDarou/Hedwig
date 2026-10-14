use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Entry {
    pub file_id: String,
    pub path: String,
    pub version_id: String,
    pub local_hash: String,
}

pub struct State {
    connection: Connection,
}

impl State {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open(path).with_context(|| {
            format!("Sync-Zustand {} kann nicht geöffnet werden", path.display())
        })?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS entries(file_id TEXT PRIMARY KEY,path TEXT NOT NULL UNIQUE,version_id TEXT NOT NULL,local_hash TEXT NOT NULL,updated_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
        Ok(Self { connection })
    }
    pub fn entries(&self) -> anyhow::Result<Vec<Entry>> {
        let mut statement = self
            .connection
            .prepare("SELECT file_id,path,version_id,local_hash FROM entries")?;
        Ok(statement
            .query_map([], |row| {
                Ok(Entry {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    version_id: row.get(2)?,
                    local_hash: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
    pub fn put(&self, entry: &Entry) -> anyhow::Result<()> {
        self.connection.execute("INSERT INTO entries(file_id,path,version_id,local_hash,updated_at) VALUES (?1,?2,?3,?4,?5)
            ON CONFLICT(file_id) DO UPDATE SET path=excluded.path,version_id=excluded.version_id,local_hash=excluded.local_hash,updated_at=excluded.updated_at",
            params![entry.file_id,entry.path,entry.version_id,entry.local_hash,chrono::Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn put_many(&self, entries: &[Entry]) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let transaction = self.connection.unchecked_transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        for entry in entries {
            transaction.execute("INSERT INTO entries(file_id,path,version_id,local_hash,updated_at) VALUES (?1,?2,?3,?4,?5)
                ON CONFLICT(file_id) DO UPDATE SET path=excluded.path,version_id=excluded.version_id,local_hash=excluded.local_hash,updated_at=excluded.updated_at",
                params![entry.file_id,entry.path,entry.version_id,entry.local_hash,now])?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn remove(&self, file_id: &str) -> anyhow::Result<()> {
        self.connection
            .execute("DELETE FROM entries WHERE file_id=?1", [file_id])?;
        Ok(())
    }
    #[allow(dead_code)]
    pub fn cursor(&self) -> anyhow::Result<i64> {
        Ok(self
            .connection
            .query_row("SELECT value FROM metadata WHERE key='cursor'", [], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }
    #[allow(dead_code)]
    pub fn set_cursor(&self, cursor: i64) -> anyhow::Result<()> {
        self.connection.execute("INSERT INTO metadata(key,value) VALUES ('cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[cursor.to_string()])?;
        Ok(())
    }
}
