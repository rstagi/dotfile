//! Rebuildable SQLite projections of session logs.
//!
//! Event bodies use these fields:
//! - `session.created`: positive integer `alias`, optional `name`, `workspace_id`, and `cwd`.
//!   Legacy imports use `path` for `cwd`.
//!   The caller obtains the alias from [`Registry::next_alias`] before the log append.
//! - `session.repo_attached`: `path`, optional `remote`, `branch`, and `base`.
//!   Repository paths identify entries. Later events update the metadata they contain.
//! - `phase.worktree`: `path` and `reference`. The reference supplies the repo's `base`.
//! - `session.renamed`: `name`. Lifecycle events use `session.<state>`.
//!   The lifecycle service validates transitions. This module projects the recorded state.
//! - `workspace.created|updated|removed`: `workspace_id`. Other fields form workspace data.
//!   Updates merge top-level fields. All mutations use the creator session's log.
//!   Removed workspace ids retain that log owner.
//! - `session.depends_on|block_on`: `target` (ULID). The event id identifies the edge.
//! - `edge.added`: `target` (ULID), `kind` (`depends_on`, `block_on`, or `parent`).
//!   The event id identifies the edge. `edge.removed` contains `edge_id`.
//! - `message.sent`: `from_session` (the log owner), `to_session`, positive `pair_seq`, and `body`.
//!   The envelope event id identifies the message. Legacy events can use `recipient`.
//!   `message.delivered|read` contains `message_id` in the recipient's log.
//!
//! Apply entries in sequence within each session. Independent logs can arrive in any order.
//! Unknown event types advance the projection cursor without changing the query tables.

use std::{collections::HashSet, fmt, fs, io, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::{DeviceId, EventId, SessionId},
    log::{LogEntry, SessionLog},
};

mod projection;

use projection::apply_entry;

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 2;

pub struct Registry {
    connection: Connection,
    alias_floor: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Session {
    pub id: SessionId,
    pub alias: i64,
    pub name: Option<String>,
    pub state: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub repos: Vec<SessionRepo>,
    pub device_id: DeviceId,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The original `session.created` body. Current values appear in the fields above.
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionRepo {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Workspace {
    pub id: String,
    pub owner_session_id: SessionId,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Edge {
    pub id: EventId,
    pub source: SessionId,
    pub target: SessionId,
    pub kind: String,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Message {
    pub id: EventId,
    pub sender: Option<SessionId>,
    pub recipient: SessionId,
    pub body: Option<Value>,
    pub sent_at_ms: Option<i64>,
    pub delivered_at_ms: Option<i64>,
    pub read_at_ms: Option<i64>,
}

#[derive(Debug, Default, PartialEq)]
pub struct RebuildReport {
    pub events: usize,
    /// Each pair contains the session id and the one-based incomplete line number.
    pub truncated_logs: Vec<(SessionId, usize)>,
}

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(io::Error),
    Json(serde_json::Error),
    InvalidEvent(String),
    NotFound(String),
    Ambiguous(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Registry {
    /// Opens the index. An older schema rebuilds from logs beside the database file.
    pub fn open(path: &Path, alias_floor: i64) -> Result<Self> {
        if alias_floor < 1 {
            return Err(Error::InvalidEvent(
                "The alias floor must be positive.".into(),
            ));
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::InvalidEvent(
                "The registry schema is newer than this binary supports.".into(),
            ));
        }
        let mut entries = Vec::new();
        if version < SCHEMA_VERSION {
            let has_sessions: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions')",
                [], |row| row.get(0),
            )?;
            if has_sessions {
                let state = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                entries = read_logs(state)?.0;
            }
            tx.execute_batch(
                "DROP TABLE IF EXISTS aliases; DROP TABLE IF EXISTS sessions;
                DROP TABLE IF EXISTS workspaces; DROP TABLE IF EXISTS edges;
                DROP TABLE IF EXISTS messages; DROP TABLE IF EXISTS applied_events;",
            )?;
        }
        tx.execute_batch(SCHEMA)?;
        for entry in &entries {
            apply_entry(&tx, entry)?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(Self {
            connection,
            alias_floor,
        })
    }

    /// Applies a verified log entry atomically. Repeat applications have no effect.
    pub fn apply(&mut self, entry: &LogEntry) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        apply_entry(&tx, entry)?;
        tx.commit()?;
        Ok(())
    }

    /// Finds a callback event in the rebuildable index.
    pub fn event_for_callback(
        &self,
        session: SessionId,
        event_type: &str,
        callback_id: &str,
    ) -> Result<Option<LogEntry>> {
        let entry: Option<String> = self.connection.query_row(
            "SELECT entry FROM applied_events WHERE session_id = ?1 AND json_extract(entry, '$.type') = ?2 AND json_extract(entry, '$.body.callback_id') = ?3 ORDER BY seq LIMIT 1",
            params![session.to_string(), event_type, callback_id], |row| row.get(0),
        ).optional()?;
        entry
            .map(|entry| serde_json::from_str(&entry).map_err(Error::Json))
            .transpose()
    }

    /// Reads one event type from the index without scanning unrelated log entries.
    pub fn events_of_type(&self, event_type: &str) -> Result<Vec<LogEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT entry FROM applied_events WHERE json_extract(entry, '$.type') = ?1 ORDER BY session_id, seq",
        )?;
        let rows = statement.query_map([event_type], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Reads one session's events of this type in sequence order.
    pub fn session_events_of_type(
        &self,
        session: SessionId,
        event_type: &str,
    ) -> Result<Vec<LogEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT entry FROM applied_events WHERE json_extract(entry, '$.type') = ?1 AND session_id = ?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![event_type, session.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Returns the latest recorded event of this type for a session.
    pub fn latest_event(&self, session: SessionId, event_type: &str) -> Result<Option<LogEntry>> {
        let entry: Option<String> = self.connection.query_row(
            "SELECT entry FROM applied_events WHERE session_id = ?1 AND json_extract(entry, '$.type') = ?2 ORDER BY seq DESC LIMIT 1",
            params![session.to_string(), event_type], |row| row.get(0),
        ).optional()?;
        entry
            .map(|entry| serde_json::from_str(&entry).map_err(Error::Json))
            .transpose()
    }

    /// Returns the latest exit recorded by a supervisor or launch wrapper.
    /// Hook callbacks do not prove that the operating system reaped the process.
    pub fn latest_process_exit(&self, session: SessionId) -> Result<Option<LogEntry>> {
        let entry: Option<String> = self.connection.query_row(
            "SELECT entry FROM applied_events WHERE session_id = ?1 AND json_extract(entry, '$.type') = 'exit' AND json_type(entry, '$.body.callback_id') IS NULL ORDER BY seq DESC LIMIT 1",
            [session.to_string()], |row| row.get(0),
        ).optional()?;
        entry
            .map(|entry| serde_json::from_str(&entry).map_err(Error::Json))
            .transpose()
    }

    /// Replaces the index from verified logs in `state/sessions`. Leaves logs unchanged.
    /// A failed rebuild preserves the previous index. Reports incomplete final log lines.
    pub fn rebuild(&mut self, state: &Path) -> Result<RebuildReport> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("DELETE FROM aliases; DELETE FROM sessions; DELETE FROM workspaces; DELETE FROM edges; DELETE FROM messages; DELETE FROM applied_events;")?;
        let mut report = RebuildReport::default();
        // Read one log at a time. Memory holds the largest log, not every log.
        for session in session_directories(state)? {
            for entry in read_log(state, session, &mut report)? {
                apply_entry(&tx, &entry)?;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// Applies only the verified log entries that the index lacks. Passes each non-empty log to `visit` once.
    /// Returns `None` and leaves the index unchanged when the index is not a prefix of the logs.
    /// The caller then discards what `visit` received and calls [`Self::rebuild`].
    pub fn catch_up(
        &mut self,
        state: &Path,
        mut visit: impl FnMut(&[LogEntry]) -> Result<()>,
    ) -> Result<Option<RebuildReport>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sessions = session_directories(state)?;
        let logged: HashSet<String> = sessions.iter().map(ToString::to_string).collect();
        let indexed = tx
            .prepare("SELECT DISTINCT session_id FROM applied_events")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        if !indexed.is_subset(&logged) {
            return Ok(None);
        }
        let mut report = RebuildReport::default();
        for session in sessions {
            let entries = read_log(state, session, &mut report)?;
            let last: Option<(i64, String)> = tx
                .query_row(
                    "SELECT seq, entry FROM applied_events WHERE session_id = ?1 ORDER BY seq DESC LIMIT 1",
                    [session.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            // Hash links fix every earlier entry. An equal last entry proves an equal prefix.
            let applied = match last {
                None => 0,
                Some((seq, indexed)) => {
                    let seq = usize::try_from(seq).unwrap_or(usize::MAX);
                    match seq.checked_sub(1).and_then(|index| entries.get(index)) {
                        Some(entry) if serde_json::to_string(entry)? == indexed => seq,
                        _ => return Ok(None),
                    }
                }
            };
            for entry in &entries[applied..] {
                apply_entry(&tx, entry)?;
            }
            if !entries.is_empty() {
                visit(&entries)?;
            }
        }
        tx.commit()?;
        Ok(Some(report))
    }

    /// Returns the next alias. Persist it in `session.created.body.alias` before apply.
    /// The caller must serialize allocation, log append, and apply.
    pub fn next_alias(&self) -> Result<i64> {
        let maximum: Option<i64> =
            self.connection
                .query_row("SELECT MAX(alias) FROM aliases", [], |row| row.get(0))?;
        match maximum {
            Some(maximum) => maximum
                .checked_add(1)
                .map(|next| next.max(self.alias_floor))
                .ok_or_else(|| Error::InvalidEvent("The alias range is exhausted.".into())),
            None => Ok(self.alias_floor),
        }
    }

    /// Resolves an alias, an exact name, or a case-insensitive ULID prefix.
    /// Exact ids precede names. Exact names precede prefixes. Ambiguous names also fail.
    pub fn resolve(&self, input: &str) -> Result<SessionId> {
        if let Some(alias) = input.strip_prefix('#') {
            if alias.is_empty()
                || !alias.bytes().all(|byte| byte.is_ascii_digit())
                || alias.parse::<i64>().is_err()
            {
                return Err(Error::NotFound(input.into()));
            }
            return select_session(
                &self.connection,
                "SELECT session_id FROM aliases WHERE alias = ?1",
                alias,
                input,
            )?
            .ok_or_else(|| Error::NotFound(input.into()));
        }
        let upper = input.to_ascii_uppercase();
        if let Some(id) = select_session(
            &self.connection,
            "SELECT id FROM sessions WHERE id = ?1",
            &upper,
            input,
        )? {
            return Ok(id);
        }
        if let Some(id) = select_session(
            &self.connection,
            "SELECT id FROM sessions WHERE name = ?1 LIMIT 2",
            input,
            input,
        )? {
            return Ok(id);
        }
        let valid_prefix = !upper.is_empty()
            && upper.len() <= 26
            && upper
                .bytes()
                .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte));
        if valid_prefix
            && let Some(id) = select_session(
                &self.connection,
                "SELECT id FROM sessions WHERE substr(id, 1, length(?1)) = ?1 LIMIT 2",
                &upper,
                input,
            )?
        {
            return Ok(id);
        }
        Err(Error::NotFound(input.into()))
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        self.query_sessions(None)
    }

    /// Reads one session without a scan of all sessions.
    pub fn session(&self, id: SessionId) -> Result<Option<Session>> {
        Ok(self.query_sessions(Some(id))?.pop())
    }

    fn query_sessions(&self, id: Option<SessionId>) -> Result<Vec<Session>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, a.alias, s.name, s.state, s.workspace_id, s.device_id, s.created_at_ms, s.updated_at_ms, s.body,
             (SELECT json_extract(entry, '$.body.code') FROM applied_events
              WHERE session_id = s.id AND json_extract(entry, '$.type') = 'exit' ORDER BY seq DESC LIMIT 1), s.cwd, s.repos
             FROM sessions s JOIN aliases a ON a.session_id = s.id
             WHERE ?1 IS NULL OR s.id = ?1 ORDER BY a.alias")?;
        let rows = statement.query_map([id.map(|id| id.to_string())], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get::<_, String>(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<i32>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                alias,
                name,
                state,
                workspace_id,
                device_id,
                created_at_ms,
                updated_at_ms,
                body,
                exit_code,
                cwd,
                repos,
            ) = row?;
            Ok(Session {
                id: parse_id(&id)?,
                alias,
                name,
                state,
                exit_code,
                workspace_id,
                cwd,
                repos: serde_json::from_str(&repos)?,
                device_id: parse_id(&device_id)?,
                created_at_ms,
                updated_at_ms,
                body: serde_json::from_str(&body)?,
            })
        })
        .collect()
    }
    pub fn messages(&self) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare("SELECT id, sender, recipient, body, sent_at_ms, delivered_at_ms, read_at_ms FROM messages ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?;
        rows.map(|row| {
            let (id, sender, recipient, body, sent_at_ms, delivered_at_ms, read_at_ms) = row?;
            Ok(Message {
                id: parse_id(&id)?,
                sender: sender.as_deref().map(parse_id).transpose()?,
                recipient: parse_id(&recipient)?,
                body: body.as_deref().map(serde_json::from_str).transpose()?,
                sent_at_ms,
                delivered_at_ms,
                read_at_ms,
            })
        })
        .collect()
    }

    pub fn edges(&self) -> Result<Vec<Edge>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, source, target, kind, body FROM edges ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, source, target, kind, body) = row?;
            Ok(Edge {
                id: parse_id(&id)?,
                source: parse_id(&source)?,
                target: parse_id(&target)?,
                kind,
                body: serde_json::from_str(&body)?,
            })
        })
        .collect()
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let mut statement = self.connection.prepare(
            "SELECT id, owner_session_id, body FROM workspaces WHERE removed = 0 ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, owner, body) = row?;
            Ok(Workspace {
                id,
                owner_session_id: parse_id(&owner)?,
                body: serde_json::from_str(&body)?,
            })
        })
        .collect()
    }

    /// Returns the log owner, including when the workspace is removed.
    pub fn workspace_owner(&self, id: &str) -> Result<Option<SessionId>> {
        select_session(
            &self.connection,
            "SELECT owner_session_id FROM workspaces WHERE id = ?1",
            id,
            id,
        )
    }
}

fn read_logs(state: &Path) -> Result<(Vec<LogEntry>, RebuildReport)> {
    let mut entries = Vec::new();
    let mut report = RebuildReport::default();
    for session in session_directories(state)? {
        entries.extend(read_log(state, session, &mut report)?);
    }
    Ok((entries, report))
}

/// Session ids of the log directories in `state/sessions`, in name order.
fn session_directories(state: &Path) -> Result<Vec<SessionId>> {
    let mut logs = match fs::read_dir(state.join("sessions")) {
        Ok(entries) => entries.collect::<std::result::Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    logs.sort_by_key(|directory| directory.file_name());
    let mut sessions = Vec::new();
    for directory in logs {
        if !directory.file_type()?.is_dir() {
            continue;
        }
        let name = directory.file_name();
        let name = name.to_str().ok_or_else(|| {
            Error::InvalidEvent("The session directory name is not UTF-8.".into())
        })?;
        let session = parse_id(name)?;
        if session.to_string() != name {
            return Err(Error::InvalidEvent(format!(
                "The session directory name is not canonical: {name}."
            )));
        }
        sessions.push(session);
    }
    Ok(sessions)
}

/// Reads one verified log. Counts its entries and records an incomplete final line in `report`.
fn read_log(state: &Path, session: SessionId, report: &mut RebuildReport) -> Result<Vec<LogEntry>> {
    let read = SessionLog::new(state, session, DeviceId::from(0_u128)).read()?;
    if let Some(line) = read.truncated_line {
        report.truncated_logs.push((session, line));
    }
    report.events += read.entries.len();
    Ok(read.entries)
}

fn select_session(
    connection: &Connection,
    sql: &str,
    parameter: &str,
    input: &str,
) -> Result<Option<SessionId>> {
    let mut statement = connection.prepare(sql)?;
    let ids = statement
        .query_map([parameter], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(parse_id(id)?)),
        _ => Err(Error::Ambiguous(input.into())),
    }
}

fn parse_id(value: &str) -> Result<SessionId> {
    value
        .parse()
        .map_err(|_| Error::InvalidEvent(format!("Invalid ULID: {value}.")))
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::InvalidEvent(message) => write!(formatter, "{message}"),
            Self::NotFound(input) => write!(formatter, "No session matches {input}."),
            Self::Ambiguous(input) => write!(formatter, "Multiple sessions match {input}."),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests;
