//! Reduces verified log entries into query tables.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::{Error, Result, SessionRepo, parse_id};
use crate::{
    ids::{EventId, SessionId},
    log::LogEntry,
};

pub(super) fn apply_entry(connection: &Connection, entry: &LogEntry) -> Result<()> {
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
    if entry.event_type == "done" {
        let changed = connection.execute(
            "UPDATE sessions SET state = 'finished', updated_at_ms = ?1 WHERE id = ?2",
            params![sql_number(entry.ts_ms)?, entry.session_id.to_string()],
        )?;
        require_session(changed, entry)?;
    }
    match entry.event_type.split_once('.') {
        Some(("session", _)) => project_session(connection, entry)?,
        Some(("phase", "worktree")) => project_repo(connection, entry)?,
        Some(("workspace", _)) => project_workspace(connection, entry)?,
        Some(("edge", _)) => project_edge(connection, entry)?,
        Some(("message", _)) => project_message(connection, entry)?,
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

fn project_session(connection: &Connection, entry: &LogEntry) -> Result<()> {
    match entry.event_type.as_str() {
        "session.depends_on" | "session.block_on" => project_edge(connection, entry)?,
        "session.created" => {
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
                "INSERT INTO sessions (id, name, state, workspace_id, device_id, created_at_ms, updated_at_ms, body, cwd)
                 VALUES (?1, ?2, 'created', ?3, ?4, ?5, ?5, ?6, ?7)",
                params![
                    entry.session_id.to_string(),
                    optional_str(entry, "name")?,
                    optional_str(entry, "workspace_id")?,
                    entry.device_id.to_string(),
                    sql_number(entry.ts_ms)?,
                    entry.body.to_string(),
                    optional_str(entry, "cwd")?.or_else(|| {
                        entry.body.get("path").and_then(Value::as_str).filter(|path| !path.is_empty())
                    }),
                ],
            )?;
            connection.execute(
                "INSERT INTO aliases (alias, session_id) VALUES (?1, ?2)",
                params![alias, entry.session_id.to_string()],
            )?;
        }
        "session.repo_attached" => project_repo(connection, entry)?,
        "session.renamed" => {
            let name = required_str(entry, "name")?;
            let changed = connection.execute(
                "UPDATE sessions SET name = ?1, updated_at_ms = ?2 WHERE id = ?3",
                params![name, sql_number(entry.ts_ms)?, entry.session_id.to_string()],
            )?;
            require_session(changed, entry)?;
        }
        "session.reparented" => {
            let parent = entry.body.get("parent").cloned().unwrap_or(Value::Null);
            if !(parent.is_null() || parent.is_string()) {
                return Err(Error::InvalidEvent(
                    "The session.reparented event requires a string or null parent.".into(),
                ));
            }
            let changed = connection.execute(
                "UPDATE sessions SET body = json_set(body, '$.parent_session_id', json(?1)), updated_at_ms = ?2 WHERE id = ?3",
                params![parent.to_string(), sql_number(entry.ts_ms)?, entry.session_id.to_string()],
            )?;
            require_session(changed, entry)?;
        }
        "session.priority" => {
            let priority = entry.body.get("priority").cloned().unwrap_or(Value::Null);
            if serde_json::from_value::<crate::config::AgentPriority>(priority.clone()).is_err() {
                return Err(Error::InvalidEvent(
                    "The session.priority event requires background, utility or normal.".into(),
                ));
            }
            let changed = connection.execute(
                "UPDATE sessions SET body = json_set(body, '$.priority', ?1), updated_at_ms = ?2 WHERE id = ?3",
                params![priority.as_str(), sql_number(entry.ts_ms)?, entry.session_id.to_string()],
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
    Ok(())
}

fn project_repo(connection: &Connection, entry: &LogEntry) -> Result<()> {
    let repo = SessionRepo {
        path: required_str(entry, "path")?.into(),
        remote: optional_str(entry, "remote")?.map(str::to_owned),
        branch: optional_str(entry, "branch")?.map(str::to_owned),
        base: if entry.event_type == "phase.worktree" {
            Some(required_str(entry, "reference")?.into())
        } else {
            optional_str(entry, "base")?.map(str::to_owned)
        },
    };
    let stored: Option<String> = connection
        .query_row(
            "SELECT repos FROM sessions WHERE id = ?1",
            [entry.session_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(stored) = stored else {
        return require_session(0, entry);
    };
    let mut repos: Vec<SessionRepo> = serde_json::from_str(&stored)?;
    if let Some(existing) = repos.iter_mut().find(|existing| existing.path == repo.path) {
        if repo.remote.is_some() {
            existing.remote = repo.remote;
        }
        if repo.branch.is_some() {
            existing.branch = repo.branch;
        }
        if repo.base.is_some() {
            existing.base = repo.base;
        }
    } else {
        repos.push(repo);
    }
    connection.execute(
        "UPDATE sessions SET repos = ?1, updated_at_ms = ?2 WHERE id = ?3",
        params![
            serde_json::to_string(&repos)?,
            sql_number(entry.ts_ms)?,
            entry.session_id.to_string()
        ],
    )?;
    Ok(())
}

fn project_workspace(connection: &Connection, entry: &LogEntry) -> Result<()> {
    match entry.event_type.as_str() {
        "workspace.created" => {
            let id = required_str(entry, "workspace_id")?;
            let owner: Option<String> = connection
                .query_row(
                    "SELECT owner_session_id FROM workspaces WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            if owner.is_some() {
                check_workspace_owner(connection, id, entry.session_id)?;
            }
            let changed = connection.execute(
                "INSERT INTO workspaces (id, owner_session_id, body) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET body = excluded.body, removed = 0 WHERE workspaces.removed = 1",
                params![id, entry.session_id.to_string(), entry.body.to_string()],
            )?;
            if changed == 0 {
                return Err(Error::InvalidEvent(format!(
                    "Workspace {id} already exists."
                )));
            }
        }
        "workspace.updated" => {
            let id = required_str(entry, "workspace_id")?;
            check_workspace_owner(connection, id, entry.session_id)?;
            let stored: Option<String> = connection
                .query_row(
                    "SELECT body FROM workspaces WHERE id = ?1 AND removed = 0",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(stored) = stored else {
                return Err(Error::InvalidEvent(format!(
                    "Workspace {id} does not exist."
                )));
            };
            let mut body: serde_json::Map<String, Value> = serde_json::from_str(&stored)?;
            body.extend(entry.body.as_object().unwrap().clone());
            connection.execute(
                "UPDATE workspaces SET body = ?1 WHERE id = ?2",
                params![serde_json::to_string(&body)?, id],
            )?;
        }
        "workspace.removed" => {
            let id = required_str(entry, "workspace_id")?;
            check_workspace_owner(connection, id, entry.session_id)?;
            connection.execute("UPDATE workspaces SET removed = 1 WHERE id = ?1", [id])?;
        }
        _ => {}
    }
    Ok(())
}

fn project_edge(connection: &Connection, entry: &LogEntry) -> Result<()> {
    match entry.event_type.as_str() {
        "edge.action_registered" => {
            for edge in entry.body["edges"].as_array().ok_or_else(|| {
                Error::InvalidEvent("The trigger requires dependency edges.".into())
            })? {
                let field = |name: &str| -> Result<String> {
                    let value = edge[name].as_str().ok_or_else(|| {
                        Error::InvalidEvent("The trigger edge requires an id.".into())
                    })?;
                    Ok(parse_id(value)?.to_string())
                };
                connection.execute("INSERT INTO edges (id, source, target, kind, body) VALUES (?1, ?2, ?3, 'block_on', ?4)",
                    params![field("id")?, field("source")?, field("target")?, edge.to_string()])?;
            }
        }
        "edge.action_removed" | "edge.action_fired" | "edge.action_failed" => {
            remove_action_edges(connection, entry)?
        }
        "edge.added" | "session.depends_on" | "session.block_on" => {
            let target = parse_id(required_str(entry, "target")?)?;
            let kind = match entry.event_type.as_str() {
                "session.depends_on" => "depends_on",
                "session.block_on" => "block_on",
                _ => required_str(entry, "kind")?,
            };
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
        _ => {}
    }
    Ok(())
}

fn remove_action_edges(connection: &Connection, entry: &LogEntry) -> Result<()> {
    let id = required_str(entry, "trigger_id")?;
    connection.execute("DELETE FROM edges WHERE json_extract(body, '$.trigger_id') = ?1 AND json_extract(body, '$.owner') = ?2",
        params![id, entry.session_id.to_string()])?;
    Ok(())
}

fn project_message(connection: &Connection, entry: &LogEntry) -> Result<()> {
    match entry.event_type.as_str() {
        "message.sent" => {
            if entry.body.get("trigger_id").is_some() {
                remove_action_edges(connection, entry)?;
            }
            let recipient = parse_id(required_str(
                entry,
                if entry.body.get("to_session").is_some() {
                    "to_session"
                } else {
                    "recipient"
                },
            )?)?;
            if entry.body.get("to_session").is_some() {
                validate_message(connection, entry, recipient)?;
            }
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
        _ => {}
    }
    Ok(())
}

fn validate_message(connection: &Connection, entry: &LogEntry, recipient: SessionId) -> Result<()> {
    if parse_id(required_str(entry, "from_session")?)? != entry.session_id {
        return Err(Error::InvalidEvent(
            "The message sender does not match the log owner.".into(),
        ));
    }
    if entry.body.get("recipient").is_some()
        && parse_id(required_str(entry, "recipient")?)? != recipient
    {
        return Err(Error::InvalidEvent(
            "The message has conflicting recipients.".into(),
        ));
    }
    let pair_seq = entry
        .body
        .get("pair_seq")
        .and_then(Value::as_u64)
        .filter(|seq| *seq > 0 && *seq < (1 << 53))
        .ok_or_else(|| {
            Error::InvalidEvent("The message pair sequence must be a positive safe integer.".into())
        })?;
    required_str(entry, "body")?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE sender = ?1 AND recipient = ?2 AND json_extract(body, '$.pair_seq') = ?3)",
        params![entry.session_id.to_string(), recipient.to_string(), sql_number(pair_seq)?],
        |row| row.get(0),
    )?;
    if exists {
        return Err(Error::InvalidEvent(
            "The message pair sequence belongs to another event.".into(),
        ));
    }
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

fn optional_str<'a>(entry: &'a LogEntry, field: &str) -> Result<Option<&'a str>> {
    match entry.body.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(Error::InvalidEvent(format!(
            "The {} event requires {field} as a nonempty string or null.",
            entry.event_type
        ))),
    }
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
