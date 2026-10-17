//! One-time import of the zsh session state.

use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ws_core::{
    ids::{SessionId, Ulid},
    log::SessionLog,
};

use crate::sessions::Sessions;

pub mod backend;
mod locks;
mod snapshot;
mod triggers;
mod workspaces;

use snapshot::snapshot;
pub use snapshot::{EventMapping, LegacySession};
pub use triggers::LegacyTrigger;
pub use workspaces::LegacyWorkspace;

const JOURNAL: &str = "import-zsh.json";

#[derive(Debug, Deserialize, Serialize)]
pub struct Plan {
    pub source: PathBuf,
    pub alias_floor: i64,
    pub sessions: Vec<LegacySession>,
    #[serde(default)]
    pub triggers: Vec<LegacyTrigger>,
    #[serde(default)]
    pub workspaces: Vec<LegacyWorkspace>,
    #[serde(default)]
    pub complete: bool,
}

/// Opens the daemon state and completes its first legacy import before serving requests.
pub fn open_for_daemon(state: &Path, home: &Path) -> io::Result<Sessions> {
    let legacy_exists = home.join("sessions").is_dir();
    let mut sessions = Sessions::open_with_home(state, 1, home).map_err(io::Error::other)?;
    let journal = load(state)?;
    let import = match journal {
        Some(plan) => !plan.complete,
        None => legacy_exists && !has_session_state(state)?,
    };
    if import {
        incremental(&mut sessions, home, &workspace_root())?;
    }
    Ok(sessions)
}

/// Imports one durable snapshot. A completed journal makes subsequent calls read-only.
pub fn from_zsh(source: &Path, state: &Path, dry_run: bool) -> io::Result<Plan> {
    from_zsh_with_root(source, state, &workspace_root(), dry_run)
}

/// Imports a snapshot with an explicit workspace root.
pub fn from_zsh_with_root(
    source: &Path,
    state: &Path,
    root: &Path,
    dry_run: bool,
) -> io::Result<Plan> {
    let source = source.canonicalize()?;
    let stored = load(state)?;
    if let Some(plan) = &stored {
        if plan.source != source {
            return Err(io::Error::other(
                "The state already contains an import from another zsh directory.",
            ));
        }
        if plan.complete || dry_run {
            return Ok(stored.unwrap());
        }
    }
    let mut plan = match stored {
        Some(plan) => plan,
        None => snapshot(&source, root, dry_run)?,
    };
    if dry_run {
        return Ok(plan);
    }
    let mut sessions =
        Sessions::open_with_home(state, plan.alias_floor, &source).map_err(io::Error::other)?;
    // Another importer can finish after the first journal read and before this lock.
    if let Some(current) = load(state)? {
        if current.source != source {
            return Err(io::Error::other(
                "The state already contains an import from another zsh directory.",
            ));
        }
        if current.complete {
            return Ok(current);
        }
        plan = current;
    } else {
        // Save the assigned ids before the first append so a crash cannot allocate replacements.
        for session in &plan.sessions {
            if sessions
                .registry
                .resolve(&format!("#{}", session.alias))
                .is_ok()
            {
                return Err(io::Error::other(format!(
                    "The alias #{} already exists in the Rust state.",
                    session.alias
                )));
            }
        }
        save(state, &plan)?;
    }
    for session in &mut plan.sessions {
        import_session(&mut sessions, session)?;
    }
    workspaces::import(&mut sessions, &plan.workspaces)?;
    triggers::import(
        &mut sessions,
        &plan.sessions,
        &plan.triggers,
        &plan.workspaces,
    )?;
    plan.complete = true;
    save(state, &plan)?;
    Ok(plan)
}

/// Imports the next zsh snapshot while the daemon owns its session writer.
pub fn incremental(sessions: &mut Sessions, source: &Path, root: &Path) -> io::Result<Plan> {
    let source = source.canonicalize()?;
    let previous = load(&sessions.state)?;
    if previous.as_ref().is_some_and(|plan| plan.source != source) {
        return Err(io::Error::other(
            "The state contains an import from another zsh directory.",
        ));
    }
    let resumed = previous.as_ref().is_some_and(|plan| !plan.complete);
    let mut plan = if let Some(previous) = previous.as_ref().filter(|plan| !plan.complete) {
        serde_json::from_slice(&serde_json::to_vec(previous)?)?
    } else {
        let mut next = snapshot(&source, root, false)?;
        for session in &mut next.sessions {
            if let Some(old) = previous
                .as_ref()
                .and_then(|p| p.sessions.iter().find(|s| s.alias == session.alias))
            {
                session.id = old.id;
                let mut available: Vec<_> = old.messages.iter().collect();
                for message in &mut session.messages {
                    if let Some(index) = available.iter().position(|old| old.raw == message.raw) {
                        let old = available.remove(index);
                        message.import_key = Some(old.import_key.clone().unwrap_or_else(|| {
                            format!("{}:{}:{}", session.alias, old.artifact, old.line)
                        }));
                    } else {
                        message.import_key = Some(format!("message:{}", Ulid::new()));
                    }
                }
            } else if sessions
                .registry
                .resolve(&format!("#{}", session.alias))
                .is_ok()
            {
                return Err(io::Error::other(format!(
                    "The alias #{} already exists in the Rust state.",
                    session.alias
                )));
            }
        }
        for trigger in &mut next.triggers {
            if let Some(old) = previous
                .as_ref()
                .and_then(|p| p.triggers.iter().find(|t| t.legacy_id == trigger.legacy_id))
            {
                trigger.id = old.id;
            }
        }
        next
    };
    save(&sessions.state, &plan)?;
    for session in &mut plan.sessions {
        import_session(sessions, session)?;
    }
    workspaces::import(sessions, &plan.workspaces)?;
    triggers::import(sessions, &plan.sessions, &plan.triggers, &plan.workspaces)?;
    plan.complete = true;
    save(&sessions.state, &plan)?;
    sessions.restore_triggers().map_err(io::Error::other)?;
    sessions.trigger_events_pending = true;
    if resumed {
        return incremental(sessions, &source, root);
    }
    Ok(plan)
}

pub(crate) fn alias_floor(state: &Path) -> io::Result<i64> {
    Ok(load(state)?.map_or(1, |plan| plan.alias_floor))
}

fn workspace_root() -> PathBuf {
    std::env::var_os("WS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".ws/worktrees")
        })
}

fn has_session_state(state: &Path) -> io::Result<bool> {
    match fs::read_dir(state.join("sessions")) {
        Ok(mut entries) => Ok(entries.next().transpose()?.is_some()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn load(state: &Path) -> io::Result<Option<Plan>> {
    match fs::read(state.join(JOURNAL)) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn import_session(sessions: &mut Sessions, session: &mut LegacySession) -> io::Result<()> {
    let mut body = session.meta.clone();
    body["alias"] = json!(session.alias);
    body["name"] = session.meta["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map_or(Value::Null, |name| json!(name));
    body["workspace_id"] = session.meta["workspace"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map_or(Value::Null, |name| json!(name));
    body["zsh_meta"] = session.meta.clone();
    if let Some(prompt) = &session.prompt {
        body["prompt"] = json!(String::from_utf8_lossy(prompt));
    }
    append_once(sessions, session.id, "meta", "session.created", body)?;
    let directory = sessions.state.join("sessions").join(session.id.to_string());
    for (name, bytes) in [("output.md", &session.output), ("prompt", &session.prompt)] {
        if let Some(bytes) = bytes {
            atomic_write(&directory.join(name), bytes)?;
        }
    }
    session.event_map.clear();
    for event in &session.events {
        let event_type = event.record["event"].as_str().unwrap();
        let mut body = event.record.clone();
        body["zsh_record"] = event.record.clone();
        if event_type == "done" {
            body["output"] = json!(directory.join("output.md"));
        }
        let kind = if ["done", "exit"].contains(&event_type) {
            event_type.to_owned()
        } else {
            format!("zsh.{event_type}")
        };
        let seq = append_once(
            sessions,
            session.id,
            &format!("events:{}", event.line),
            &kind,
            body,
        )?;
        session.event_map.push(EventMapping {
            line: event.line,
            raw: event.raw.clone(),
            seq,
        });
    }
    if session.meta["status"] == "closed" {
        append_once(sessions, session.id, "status", "session.failed", json!({}))?;
    } else if session.meta["status"] == "done" || session.output.is_some() {
        append_once(
            sessions,
            session.id,
            "status",
            "session.finished",
            json!({}),
        )?;
        if !session
            .events
            .iter()
            .any(|event| event.record["event"] == "done")
        {
            append_once(
                sessions,
                session.id,
                "output",
                "done",
                json!({"output":directory.join("output.md")}),
            )?;
        }
    }
    if let Some(paste) = &session.paste {
        append_once(
            sessions,
            session.id,
            "paste",
            "zsh.paste.pending",
            paste.clone(),
        )?;
    }
    if !session.messages.is_empty() {
        let human = sessions.message_session(None).map_err(io::Error::other)?;
        for message in &session.messages {
            let pair_seq = sessions
                .registry
                .messages()
                .map_err(io::Error::other)?
                .iter()
                .filter(|message| {
                    message.sender == Some(human.id) && message.recipient == session.id
                })
                .filter_map(|message| message.body.as_ref()?.get("pair_seq")?.as_u64())
                .max()
                .unwrap_or(0)
                + 1;
            append_once(
                sessions,
                human.id,
                &message.import_key.clone().unwrap_or_else(|| {
                    format!("{}:{}:{}", session.alias, message.artifact, message.line)
                }),
                "message.sent",
                json!({"from_session":human.id, "to_session":session.id, "pair_seq":pair_seq, "body":message.record["message"], "zsh_artifact":message.artifact, "zsh_record":message.record}),
            )?;
        }
    }
    Ok(())
}

fn append_once(
    sessions: &mut Sessions,
    id: SessionId,
    key: &str,
    event_type: &str,
    mut body: Value,
) -> io::Result<u64> {
    let read = SessionLog::new(&sessions.state, id, sessions.device_id).read()?;
    if let Some(entry) = read
        .entries
        .iter()
        .find(|entry| entry.body["zsh_import_key"] == key)
    {
        return Ok(entry.seq);
    }
    body["zsh_import_key"] = json!(key);
    sessions
        .append_entry(id, event_type, body)
        .map(|entry| entry.seq)
        .map_err(io::Error::other)
}

fn save(state: &Path, plan: &Plan) -> io::Result<()> {
    atomic_write(&state.join(JOURNAL), &serde_json::to_vec_pretty(plan)?)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path.parent().unwrap();
    let temporary = directory.join(format!(".import-{}", Ulid::new()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests;
