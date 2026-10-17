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

const JOURNAL: &str = "import-zsh.json";

#[derive(Debug, Deserialize, Serialize)]
pub struct Plan {
    pub source: PathBuf,
    pub alias_floor: i64,
    pub sessions: Vec<LegacySession>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LegacySession {
    pub alias: i64,
    pub id: SessionId,
    pub meta: Value,
}

/// Imports one durable snapshot. A completed journal makes subsequent calls read-only.
pub fn from_zsh(source: &Path, state: &Path, dry_run: bool) -> io::Result<Plan> {
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
        None => snapshot(&source)?,
    };
    if dry_run {
        return Ok(plan);
    }
    let mut sessions = Sessions::open(state, plan.alias_floor).map_err(io::Error::other)?;
    // Save the assigned ids before the first append so a crash cannot allocate replacements.
    if !state.join(JOURNAL).exists() {
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
    for session in &plan.sessions {
        let mut body = session.meta.clone();
        body["alias"] = json!(session.alias);
        body["workspace_id"] = session.meta["workspace"].clone();
        body["zsh_meta"] = session.meta.clone();
        append_once(&mut sessions, session.id, "meta", "session.created", body)?;
        if session.meta["status"] == "closed" {
            append_once(
                &mut sessions,
                session.id,
                "status",
                "session.failed",
                json!({}),
            )?;
        } else if session.meta["status"] == "done" {
            append_once(
                &mut sessions,
                session.id,
                "status",
                "session.finished",
                json!({}),
            )?;
        }
    }
    plan.complete = true;
    save(state, &plan)?;
    Ok(plan)
}

pub(crate) fn alias_floor(state: &Path) -> io::Result<i64> {
    Ok(load(state)?.map_or(1, |plan| plan.alias_floor))
}

fn load(state: &Path) -> io::Result<Option<Plan>> {
    match fs::read(state.join(JOURNAL)) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn snapshot(source: &Path) -> io::Result<Plan> {
    let directory = source.join("sessions");
    let counter = fs::read_to_string(directory.join("counter"))?
        .trim()
        .parse::<i64>()
        .map_err(io::Error::other)?;
    let alias_floor = counter
        .checked_add(1)
        .filter(|floor| *floor > 0)
        .ok_or_else(|| io::Error::other("The zsh session counter is invalid."))?;
    let mut sessions = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Ok(alias) = entry.file_name().to_string_lossy().parse::<i64>() else {
            continue;
        };
        if alias < 1 || !entry.file_type()?.is_dir() {
            continue;
        }
        let meta: Value = serde_json::from_slice(&fs::read(entry.path().join("meta"))?)?;
        if !meta.is_object() || alias >= alias_floor {
            return Err(io::Error::other(
                "The zsh session metadata or counter is invalid.",
            ));
        }
        sessions.push(LegacySession {
            alias,
            id: Ulid::new(),
            meta,
        });
    }
    sessions.sort_by_key(|session| session.alias);
    Ok(Plan {
        source: source.into(),
        alias_floor,
        sessions,
        complete: false,
    })
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
