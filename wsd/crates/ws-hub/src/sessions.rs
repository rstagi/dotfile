//! The sessions table of the hub: one row for each synced session, rebuilt from its entries.
//!
//! A session waits for input from its last `agent.waiting` until its next `turn.started`.
//! The row then shows the state `blocked`. The lifecycle state of the device registry does
//! not change. A finished or failed session never shows `blocked`.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{identity::Result, sync::SyncStore};

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS hub_sessions (
    device_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    name TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    cwd TEXT,
    repos TEXT NOT NULL,
    last_event_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, session_id)
);";

/// A repository that a session attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRepo {
    pub path: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
}

/// One row of `GET /v1/sessions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubSession {
    pub id: String,
    pub device_id: String,
    /// The registry name of the device. The hub fills it in when it serves the table.
    #[serde(default)]
    pub device_name: String,
    pub name: Option<String>,
    pub kind: String,
    pub state: String,
    pub cwd: Option<String>,
    pub repos: Vec<SessionRepo>,
    pub last_event_ms: u64,
}

impl SyncStore {
    /// All rows of the sessions table, newest event first.
    pub fn sessions(&self) -> Result<Vec<HubSession>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT session_id, device_id, name, kind, state, cwd, repos, last_event_ms
             FROM hub_sessions ORDER BY last_event_ms DESC, session_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(HubSession {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    device_name: String::new(),
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    state: row.get(4)?,
                    cwd: row.get(5)?,
                    repos: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                    last_event_ms: row.get::<_, i64>(7)? as u64,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
}

/// Builds the rows of sessions that synced before the table existed.
pub(crate) fn backfill(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT device_id, session_id FROM sync_entries
         WHERE NOT EXISTS (SELECT 1 FROM hub_sessions h
            WHERE h.device_id = sync_entries.device_id AND h.session_id = sync_entries.session_id)",
    )?;
    let missing: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (device_id, session_id) in missing {
        refresh(connection, &device_id, &session_id)?;
    }
    Ok(())
}

/// Rebuilds the row of one session from its stored entries. The caller holds the transaction.
pub(crate) fn refresh(connection: &Connection, device_id: &str, session_id: &str) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT ts_ms, type, body FROM sync_entries
         WHERE device_id = ?1 AND session_id = ?2 ORDER BY seq",
    )?;
    let mut row = Row::default();
    let entries = statement.query_map(params![device_id, session_id], |entry| {
        Ok((
            entry.get::<_, i64>(0)? as u64,
            entry.get::<_, String>(1)?,
            entry.get::<_, String>(2)?,
        ))
    })?;
    for entry in entries {
        let (ts_ms, event_type, body) = entry?;
        row.apply(
            ts_ms,
            &event_type,
            &serde_json::from_str(&body).unwrap_or_default(),
        );
    }
    connection.execute(
        "INSERT INTO hub_sessions (device_id, session_id, name, kind, state, cwd, repos, last_event_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(device_id, session_id) DO UPDATE SET
            name = excluded.name, kind = excluded.kind, state = excluded.state, cwd = excluded.cwd,
            repos = excluded.repos, last_event_ms = excluded.last_event_ms",
        params![
            device_id,
            session_id,
            row.name,
            row.kind,
            row.state(),
            row.cwd,
            serde_json::to_string(&row.repos)?,
            row.last_event_ms as i64
        ],
    )?;
    Ok(())
}

struct Row {
    name: Option<String>,
    kind: String,
    cwd: Option<String>,
    repos: Vec<SessionRepo>,
    lifecycle: &'static str,
    waiting: bool,
    last_event_ms: u64,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            name: None,
            kind: "session".into(),
            cwd: None,
            repos: Vec::new(),
            lifecycle: "created",
            waiting: false,
            last_event_ms: 0,
        }
    }
}

impl Row {
    fn apply(&mut self, ts_ms: u64, event_type: &str, body: &Value) {
        self.last_event_ms = self.last_event_ms.max(ts_ms);
        match event_type {
            "session.created" => {
                self.name = text(body, "name");
                self.cwd = text(body, "cwd");
                if let Some(kind) = text(body, "kind") {
                    self.kind = kind;
                }
            }
            "session.renamed" => self.name = text(body, "name"),
            "session.repo_attached" => {
                if let Some(path) = text(body, "path") {
                    self.repos.retain(|repo| repo.path != path);
                    self.repos.push(SessionRepo {
                        path,
                        remote: text(body, "remote"),
                        branch: text(body, "branch"),
                    });
                }
            }
            "session.running" => self.lifecycle = "running",
            "session.blocked" => self.lifecycle = "blocked",
            "session.reopened" => self.lifecycle = "reopened",
            "session.finished" | "done" => self.end("finished"),
            "session.failed" => self.end("failed"),
            "agent.waiting" => self.waiting = true,
            "turn.started" => self.waiting = false,
            _ => {}
        }
    }

    fn end(&mut self, lifecycle: &'static str) {
        self.lifecycle = lifecycle;
        self.waiting = false;
    }

    fn state(&self) -> &'static str {
        if self.waiting {
            "blocked"
        } else {
            self.lifecycle
        }
    }
}

fn text(body: &Value, field: &str) -> Option<String> {
    body[field].as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{ExportEntry, SessionBatch, SyncBatch};
    use serde_json::json;

    fn push(store: &SyncStore, seq: u64, event_type: &str, body: Value) {
        let entry = ExportEntry {
            event_id: format!("E{seq}"),
            session_id: "S1".into(),
            seq,
            ts_ms: seq * 10,
            event_type: event_type.into(),
            body,
        };
        let batch = SyncBatch {
            sessions: vec![SessionBatch {
                session_id: "S1".into(),
                through_seq: seq,
                entries: vec![entry],
            }],
        };
        store.ingest("dev-a", &batch).unwrap();
    }

    fn state(store: &SyncStore) -> String {
        store.sessions().unwrap()[0].state.clone()
    }

    #[test]
    fn a_wait_blocks_the_session_until_the_next_turn_starts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::open(dir.path()).unwrap();
        push(
            &store,
            1,
            "session.created",
            json!({"name": "n", "cwd": "/w", "kind": "headless"}),
        );
        push(&store, 2, "session.running", json!({}));
        assert_eq!(state(&store), "running");
        push(&store, 3, "agent.waiting", json!({}));
        assert_eq!(state(&store), "blocked");
        push(&store, 4, "turn.ended", json!({}));
        assert_eq!(state(&store), "blocked");
        push(&store, 5, "turn.started", json!({}));
        assert_eq!(state(&store), "running");
        let row = &store.sessions().unwrap()[0];
        assert_eq!(
            (row.name.as_deref(), row.cwd.as_deref(), row.kind.as_str()),
            (Some("n"), Some("/w"), "headless")
        );
        assert_eq!(
            (row.id.as_str(), row.device_id.as_str(), row.last_event_ms),
            ("S1", "dev-a", 50)
        );
    }

    #[test]
    fn a_finished_session_is_not_blocked_and_repos_and_names_follow_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::open(dir.path()).unwrap();
        push(&store, 1, "session.created", json!({"name": "n"}));
        push(&store, 2, "agent.waiting", json!({}));
        push(&store, 3, "session.renamed", json!({"name": "m"}));
        push(
            &store,
            4,
            "session.repo_attached",
            json!({"path": "/r", "remote": "https://h/a/b", "branch": "main"}),
        );
        push(&store, 5, "session.finished", json!({}));
        let row = &store.sessions().unwrap()[0];
        assert_eq!(
            (row.state.as_str(), row.name.as_deref()),
            ("finished", Some("m"))
        );
        assert_eq!(
            row.repos,
            [SessionRepo {
                path: "/r".into(),
                remote: Some("https://h/a/b".into()),
                branch: Some("main".into())
            }]
        );
    }
}
