//! Rebuildable SQLite projections of session logs.

use std::{fmt, io, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ids::{DeviceId, SessionId},
    log::LogEntry,
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
        if valid_prefix {
            if let Some(id) = select_session(
                &self.connection,
                "SELECT id FROM sessions WHERE substr(id, 1, length(?1)) = ?1 LIMIT 2",
                &upper,
                input,
            )? {
                return Ok(id);
            }
        }
        Err(Error::NotFound(input.into()))
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
