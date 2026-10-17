//! Rebuildable SQLite projections of session logs.

use std::{fmt, fs, io, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::{DeviceId, EventId, SessionId},
    log::{LogEntry, SessionLog},
};

const SCHEMA: &str = include_str!("schema.sql");

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
    pub workspace_id: Option<String>,
    pub device_id: DeviceId,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub body: Value,
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
    pub fn open(path: &Path, alias_floor: i64) -> Result<Self> {
        if alias_floor < 1 {
            return Err(Error::InvalidEvent(
                "The alias floor must be positive.".into(),
            ));
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        connection.execute_batch(SCHEMA)?;
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

    /// Replaces the index from verified logs in `state/sessions`. Leaves logs unchanged.
    /// A failed rebuild preserves the previous index. Reports incomplete final log lines.
    pub fn rebuild(&mut self, state: &Path) -> Result<RebuildReport> {
        let mut logs = match fs::read_dir(state.join("sessions")) {
            Ok(entries) => entries.collect::<std::result::Result<Vec<_>, _>>()?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        logs.sort_by_key(|directory| directory.file_name());
        let mut entries = Vec::new();
        let mut report = RebuildReport::default();
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
            let read = SessionLog::new(state, session, DeviceId::from(0_u128)).read()?;
            if let Some(line) = read.truncated_line {
                report.truncated_logs.push((session, line));
            }
            entries.extend(read.entries);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("DELETE FROM aliases; DELETE FROM sessions; DELETE FROM workspaces; DELETE FROM edges; DELETE FROM messages; DELETE FROM applied_events;")?;
        for entry in &entries {
            apply_entry(&tx, entry)?;
        }
        report.events = entries.len();
        tx.commit()?;
        Ok(report)
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
        let mut statement = self
            .connection
            .prepare("SELECT id, owner_session_id, body FROM workspaces ORDER BY id")?;
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

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, a.alias, s.name, s.state, s.workspace_id, s.device_id, s.created_at_ms, s.updated_at_ms, s.body
             FROM sessions s JOIN aliases a ON a.session_id = s.id ORDER BY a.alias")?;
        let rows = statement.query_map([], |row| {
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
            ) = row?;
            Ok(Session {
                id: parse_id(&id)?,
                alias,
                name,
                state,
                workspace_id,
                device_id: parse_id(&device_id)?,
                created_at_ms,
                updated_at_ms,
                body: serde_json::from_str(&body)?,
            })
        })
        .collect()
    }
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

fn apply_entry(connection: &Connection, entry: &LogEntry) -> Result<()> {
    let serialized = serde_json::to_string(entry)?;
    let previous: Option<String> = connection
        .query_row(
            "SELECT entry FROM applied_events WHERE event_id = ?1",
            [entry.event_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        if previous != serialized {
            return Err(Error::InvalidEvent(
                "The event id has different content.".into(),
            ));
        }
        return Ok(());
    }
    let previous_seq: i64 = connection.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM applied_events WHERE session_id = ?1",
        [entry.session_id.to_string()],
        |row| row.get(0),
    )?;
    if sql_number(entry.seq)? != previous_seq + 1 {
        return Err(Error::InvalidEvent(
            "The event sequence is not consecutive.".into(),
        ));
    }
    if entry.event_type == "session.created" {
        let alias = entry
            .body
            .get("alias")
            .and_then(Value::as_i64)
            .filter(|alias| *alias > 0)
            .ok_or_else(|| {
                Error::InvalidEvent(
                    "The session.created event requires a positive integer alias.".into(),
                )
            })?;
        connection.execute(
            "INSERT INTO sessions (id, name, state, workspace_id, device_id, created_at_ms, updated_at_ms, body)
             VALUES (?1, ?2, 'created', ?3, ?4, ?5, ?5, ?6)",
            params![entry.session_id.to_string(), entry.body.get("name").and_then(Value::as_str),
                entry.body.get("workspace_id").and_then(Value::as_str), entry.device_id.to_string(), sql_number(entry.ts_ms)?, entry.body.to_string()])?;
        connection.execute(
            "INSERT INTO aliases (alias, session_id) VALUES (?1, ?2)",
            params![alias, entry.session_id.to_string()],
        )?;
    }
    match entry.event_type.as_str() {
        "message.sent" => {
            let recipient = parse_id(required_str(entry, "recipient")?)?;
            check_recipient(connection, entry.event_id, recipient)?;
            connection.execute(
                "INSERT INTO messages (id, sender, recipient, body, sent_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET sender = excluded.sender, body = excluded.body, sent_at_ms = excluded.sent_at_ms",
                params![entry.event_id.to_string(), entry.session_id.to_string(), recipient.to_string(), entry.body.to_string(), sql_number(entry.ts_ms)?])?;
        }
        "message.delivered" | "message.read" => {
            let id = parse_id(required_str(entry, "message_id")?)?;
            check_recipient(connection, id, entry.session_id)?;
            let column = if entry.event_type == "message.delivered" {
                "delivered_at_ms"
            } else {
                "read_at_ms"
            };
            connection.execute(&format!(
                "INSERT INTO messages (id, recipient, {column}) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET {column} = MIN(COALESCE({column}, excluded.{column}), excluded.{column})"),
                params![id.to_string(), entry.session_id.to_string(), sql_number(entry.ts_ms)?])?;
        }
        "edge.added" => {
            let target = parse_id(required_str(entry, "target")?)?;
            let kind = required_str(entry, "kind")?;
            if !["depends_on", "block_on", "parent"].contains(&kind) {
                return Err(Error::InvalidEvent(format!("Invalid edge kind: {kind}.")));
            }
            connection.execute(
                "INSERT INTO edges (id, source, target, kind, body) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entry.event_id.to_string(),
                    entry.session_id.to_string(),
                    target.to_string(),
                    kind,
                    entry.body.to_string()
                ],
            )?;
        }
        "edge.removed" => {
            let id = parse_id(required_str(entry, "edge_id")?)?;
            connection.execute(
                "DELETE FROM edges WHERE id = ?1 AND source = ?2",
                params![id.to_string(), entry.session_id.to_string()],
            )?;
        }
        "workspace.created" => {
            let id = required_str(entry, "workspace_id")?;
            connection.execute(
                "INSERT INTO workspaces (id, owner_session_id, body) VALUES (?1, ?2, ?3)",
                params![id, entry.session_id.to_string(), entry.body.to_string()],
            )?;
        }
        "workspace.updated" => {
            let id = required_str(entry, "workspace_id")?;
            check_workspace_owner(connection, id, entry.session_id)?;
            let stored: Option<String> = connection
                .query_row("SELECT body FROM workspaces WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .optional()?;
            let Some(stored) = stored else {
                return Err(Error::InvalidEvent(format!(
                    "Workspace {id} does not exist."
                )));
            };
            let mut body: Value = serde_json::from_str(&stored)?;
            body.as_object_mut()
                .unwrap()
                .extend(entry.body.as_object().unwrap().clone());
            connection.execute(
                "UPDATE workspaces SET body = ?1 WHERE id = ?2",
                params![body.to_string(), id],
            )?;
        }
        "workspace.removed" => {
            let id = required_str(entry, "workspace_id")?;
            check_workspace_owner(connection, id, entry.session_id)?;
            connection.execute("DELETE FROM workspaces WHERE id = ?1", [id])?;
        }
        "session.renamed" => {
            let name = required_str(entry, "name")?;
            let changed = connection.execute(
                "UPDATE sessions SET name = ?1, updated_at_ms = ?2 WHERE id = ?3",
                params![name, sql_number(entry.ts_ms)?, entry.session_id.to_string()],
            )?;
            require_session(changed, entry)?;
        }
        "session.running" | "session.blocked" | "session.finished" | "session.failed"
        | "session.reopened" => {
            let state = entry.event_type.strip_prefix("session.").unwrap();
            let changed = connection.execute(
                "UPDATE sessions SET state = ?1, device_id = ?2, updated_at_ms = ?3 WHERE id = ?4",
                params![
                    state,
                    entry.device_id.to_string(),
                    sql_number(entry.ts_ms)?,
                    entry.session_id.to_string()
                ],
            )?;
            require_session(changed, entry)?;
        }
        _ => {}
    }
    connection.execute(
        "INSERT INTO applied_events (event_id, session_id, seq, entry) VALUES (?1, ?2, ?3, ?4)",
        params![
            entry.event_id.to_string(),
            entry.session_id.to_string(),
            sql_number(entry.seq)?,
            serialized
        ],
    )?;
    Ok(())
}

fn check_workspace_owner(connection: &Connection, id: &str, session: SessionId) -> Result<()> {
    let owner: Option<String> = connection
        .query_row(
            "SELECT owner_session_id FROM workspaces WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    match owner {
        Some(owner) if owner == session.to_string() => Ok(()),
        Some(owner) => Err(Error::InvalidEvent(format!(
            "Workspace {id} belongs to session {owner}."
        ))),
        None => Err(Error::InvalidEvent(format!(
            "Workspace {id} does not exist."
        ))),
    }
}

fn check_recipient(connection: &Connection, id: EventId, recipient: SessionId) -> Result<()> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT recipient FROM messages WHERE id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if stored.is_some_and(|stored| stored != recipient.to_string()) {
        return Err(Error::InvalidEvent(format!(
            "The recipient does not match message {id}."
        )));
    }
    Ok(())
}

fn required_str<'a>(entry: &'a LogEntry, field: &str) -> Result<&'a str> {
    entry
        .body
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::InvalidEvent(format!(
                "The {} event requires {field} as a nonempty string.",
                entry.event_type
            ))
        })
}

fn require_session(changed: usize, entry: &LogEntry) -> Result<()> {
    if changed == 0 {
        return Err(Error::InvalidEvent(format!(
            "Session {} does not exist.",
            entry.session_id
        )));
    }
    Ok(())
}

fn sql_number(value: u64) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| Error::InvalidEvent("The integer exceeds the SQLite range.".into()))
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
