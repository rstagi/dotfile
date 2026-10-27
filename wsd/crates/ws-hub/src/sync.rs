//! Session log sync: the wire types and the hub store of exported log entries.
//!
//! A device pushes an export projection of its log entries. The hub stores each entry one
//! time by `event_id` and keeps one cursor for each device and session.

use std::{collections::BTreeMap, fs, path::Path, sync::Mutex};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{identity::Result, registry::DB_FILE};

/// The most entries in one `POST /v1/sync/logs` request.
pub const MAX_BATCH_ENTRIES: usize = 200;

/// The body of `POST /v1/sync/logs`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SyncBatch {
    pub sessions: Vec<SessionBatch>,
}

/// The exported entries of one session. The hub moves the cursor of the session to
/// `through_seq`, because the export projection skips some entries.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionBatch {
    pub session_id: String,
    pub through_seq: u64,
    pub entries: Vec<ExportEntry>,
}

/// One entry after the export projection. It has no device id: the hub takes the device id
/// from the client certificate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEntry {
    pub event_id: String,
    pub session_id: String,
    pub seq: u64,
    pub ts_ms: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub body: Value,
}

/// An entry as the hub stores it.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredEntry {
    pub device_id: String,
    #[serde(flatten)]
    pub entry: ExportEntry,
}

pub struct SyncStore {
    connection: Mutex<Connection>,
}

impl SyncStore {
    /// Opens the sync tables in the hub database of `state`.
    pub fn open(state: &Path) -> Result<Self> {
        fs::create_dir_all(state)?;
        let connection = Connection::open(state.join(DB_FILE))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_entries (
                event_id TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                ts_ms INTEGER NOT NULL,
                type TEXT NOT NULL,
                body TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS sync_entries_session ON sync_entries(device_id, session_id, seq);
            CREATE TABLE IF NOT EXISTS sync_cursors (
                device_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                PRIMARY KEY (device_id, session_id)
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Stores a batch in one transaction. Returns the number of new entries.
    pub fn ingest(&self, device_id: &str, batch: &SyncBatch) -> Result<usize> {
        let mut connection = self.connection.lock().unwrap();
        let transaction = connection.transaction()?;
        let mut stored = 0;
        for session in &batch.sessions {
            for entry in &session.entries {
                stored += transaction.execute(
                    "INSERT OR IGNORE INTO sync_entries (event_id, device_id, session_id, seq, ts_ms, type, body)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        entry.event_id,
                        device_id,
                        session.session_id,
                        entry.seq as i64,
                        entry.ts_ms as i64,
                        entry.event_type,
                        entry.body.to_string()
                    ],
                )?;
            }
            transaction.execute(
                "INSERT INTO sync_cursors (device_id, session_id, seq) VALUES (?1, ?2, ?3)
                 ON CONFLICT(device_id, session_id) DO UPDATE SET seq = MAX(seq, excluded.seq)",
                params![device_id, session.session_id, session.through_seq as i64],
            )?;
        }
        transaction.commit()?;
        Ok(stored)
    }

    /// The highest synced sequence number of each session of a device.
    pub fn cursors(&self, device_id: &str) -> Result<BTreeMap<String, u64>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT session_id, seq FROM sync_cursors WHERE device_id = ?1")?;
        let rows = statement
            .query_map([device_id], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// The stored entries of a device in session and sequence order.
    pub fn entries(&self, device_id: &str) -> Result<Vec<StoredEntry>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT event_id, session_id, seq, ts_ms, type, body FROM sync_entries
             WHERE device_id = ?1 ORDER BY session_id, seq",
        )?;
        let rows = statement
            .query_map([device_id], |row| {
                Ok(StoredEntry {
                    device_id: device_id.to_owned(),
                    entry: ExportEntry {
                        event_id: row.get(0)?,
                        session_id: row.get(1)?,
                        seq: row.get::<_, i64>(2)? as u64,
                        ts_ms: row.get::<_, i64>(3)? as u64,
                        event_type: row.get(4)?,
                        body: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                    },
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(event_id: &str, seq: u64) -> ExportEntry {
        ExportEntry {
            event_id: event_id.into(),
            session_id: "S1".into(),
            seq,
            ts_ms: 10,
            event_type: "turn.started".into(),
            body: json!({}),
        }
    }

    fn batch(entries: Vec<ExportEntry>, through_seq: u64) -> SyncBatch {
        SyncBatch {
            sessions: vec![SessionBatch {
                session_id: "S1".into(),
                through_seq,
                entries,
            }],
        }
    }

    #[test]
    fn ingest_stores_each_event_one_time_and_moves_the_cursor_forward_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::open(dir.path()).unwrap();
        assert_eq!(
            store
                .ingest("dev-a", &batch(vec![entry("E1", 1), entry("E2", 2)], 3))
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .ingest("dev-a", &batch(vec![entry("E2", 2)], 2))
                .unwrap(),
            0
        );
        assert_eq!(store.cursors("dev-a").unwrap()["S1"], 3);
        assert_eq!(store.entries("dev-a").unwrap().len(), 2);
        assert!(store.cursors("dev-b").unwrap().is_empty());
    }
}
