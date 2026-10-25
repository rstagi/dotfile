//! Guard context from durable session ancestry.

use std::{collections::BTreeSet, io, path::Path};
use ws_core::{ids::SessionId, log::{LogEntry, SessionLog}};

pub struct Context {
    pub id: SessionId,
    pub role: Option<String>,
    pub entries: Vec<LogEntry>,
}

/// Finds the nearest loop ancestor. Reads no registry and changes no state.
pub fn context(
    state: &Path,
    session: SessionId,
    parent: Option<SessionId>,
) -> io::Result<Option<Context>> {
    let mut next = Some(session);
    let mut seen = BTreeSet::new();
    let mut role = None;
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "The session parents contain a cycle."));
        }
        let entries = SessionLog::new(state, id, SessionId::default()).read()?.entries;
        let created = entries.iter().find(|entry| entry.event_type == "session.created");
        if created.is_some_and(|entry| entry.body["kind"] == "loop") {
            return Ok(Some(Context { id, role, entries }));
        }
        if role.is_none() {
            role = created.and_then(|entry| entry.body["loop"]["role"].as_str().map(str::to_owned));
        }
        next = match entries.iter().rev().find(|entry| entry.event_type == "session.reparented") {
            Some(entry) => entry.body["parent"].as_str().and_then(|id| id.parse().ok()),
            None => created.and_then(|entry| entry.body["parent_session_id"].as_str())
                .and_then(|id| id.parse().ok())
                .or_else(|| (id == session).then_some(parent).flatten()),
        };
    }
    Ok(None)
}
