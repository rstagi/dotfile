//! Converts pending zsh actions into durable Rust trigger definitions.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ws_core::ids::Ulid;

use super::{LegacySession, LegacyWorkspace, append_once, locks::Locks};
use crate::{sessions::Sessions, triggers::Trigger};

#[derive(Debug, Deserialize, Serialize)]
pub struct LegacyTrigger {
    pub legacy_id: String,
    pub id: Ulid,
    record: Record,
}

#[derive(Debug, Deserialize, Serialize)]
struct Record {
    sources: Vec<String>,
    event: String,
    action: String,
    target: String,
    template: String,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    agent: String,
    #[serde(default)]
    event_offsets: BTreeMap<String, usize>,
}

pub(super) fn snapshot(
    source: &Path,
    locks: &mut Locks,
    dry_run: bool,
) -> io::Result<Vec<LegacyTrigger>> {
    let directory = source.join("triggers");
    if !directory.try_exists()? {
        locks.acquire(&directory.join("registry.lock"), true)?;
        return Ok(Vec::new());
    }
    locks.acquire(&directory.join("registry.lock"), dry_run)?;
    let mut paths = fs::read_dir(&directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
                })
    });
    paths.sort();
    let mut triggers = Vec::new();
    for path in paths {
        locks.acquire(&path.with_extension("lock"), dry_run)?;
        let record: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if record["status"] != "pending" {
            continue;
        }
        let mut record: Record = serde_json::from_value(record)?;
        if !["done", "exit"].contains(&record.event.as_str())
            || !["send", "spawn"].contains(&record.action.as_str())
            || record.sources.is_empty()
        {
            return Err(io::Error::other("The pending zsh trigger is invalid."));
        }
        if record.action == "spawn" {
            record.path = Path::new(&record.path)
                .canonicalize()?
                .to_str()
                .ok_or_else(|| io::Error::other("The spawn workspace path is not UTF-8."))?
                .to_owned();
        }
        triggers.push(LegacyTrigger {
            legacy_id: path.file_stem().unwrap().to_string_lossy().into_owned(),
            id: Ulid::new(),
            record,
        });
    }
    Ok(triggers)
}

pub(super) fn import(
    sessions: &mut Sessions,
    legacy_sessions: &[LegacySession],
    triggers: &[LegacyTrigger],
    workspaces: &[LegacyWorkspace],
) -> io::Result<()> {
    for legacy in triggers {
        let record = &legacy.record;
        let resolve = |alias: &str| -> io::Result<&LegacySession> {
            legacy_sessions
                .iter()
                .find(|session| session.alias.to_string() == alias)
                .ok_or_else(|| {
                    io::Error::other(format!(
                        "The zsh trigger refers to missing session #{alias}."
                    ))
                })
        };
        let owner = if record.sender.is_empty() {
            sessions.message_session(None).map_err(io::Error::other)?.id
        } else {
            resolve(&record.sender)?.id
        };
        let target = if record.action == "send" {
            Some(resolve(&record.target)?.id)
        } else {
            None
        };
        let spawned_session = (record.action == "spawn").then(Ulid::new);
        let dependent = target.or(spawned_session).unwrap_or(owner);
        let mut sources = Vec::new();
        let mut offsets = BTreeMap::new();
        let mut edges = Vec::new();
        for alias in &record.sources {
            let session = resolve(alias)?;
            let offset = record.event_offsets.get(alias).copied().unwrap_or(0);
            let seq = if offset == 0 {
                0
            } else {
                session
                    .event_map
                    .iter()
                    .find(|mapping| mapping.line == offset)
                    .map(|mapping| mapping.seq)
                    .ok_or_else(|| {
                        io::Error::other("The zsh trigger offset exceeds its source event log.")
                    })?
            };
            if sources.contains(&session.id) {
                continue;
            }
            sources.push(session.id);
            offsets.insert(session.id, seq);
            if record.event != "exit" || session.id != dependent {
                edges.push(json!({"id":Ulid::new(), "source":dependent, "target":session.id, "kind":"block_on", "trigger_id":legacy.id, "owner":owner}));
            }
        }
        let workspace = if record.action == "spawn" {
            let path = Path::new(&record.path);
            Some(
                workspaces
                    .iter()
                    .find(|workspace| workspace.path == path)
                    .ok_or_else(|| {
                        io::Error::other("The spawn trigger requires a listed branch workspace.")
                    })?
                    .id
                    .clone(),
            )
        } else {
            None
        };
        let trigger = Trigger {
            id: legacy.id,
            owner,
            sources,
            event: record.event.clone(),
            action: record.action.clone(),
            target,
            template: record.template.clone(),
            agent: (record.action == "spawn").then(|| record.agent.clone()),
            workspace,
            cwd: None,
            spawned_session,
            event_offsets: offsets,
            edges,
        };
        append_once(
            sessions,
            owner,
            &format!("trigger:{}", legacy.legacy_id),
            "edge.action_registered",
            serde_json::to_value(trigger)?,
        )?;
    }
    Ok(())
}

/// Records the end of an imported trigger that left the zsh pending set.
/// A zsh `fired` record completes it. Any other outcome cancels it.
pub(super) fn retire(
    sessions: &mut Sessions,
    source: &Path,
    legacy: &LegacyTrigger,
) -> io::Result<()> {
    let Some(owner) = sessions
        .triggers
        .get(&legacy.id)
        .map(|trigger| trigger.owner)
    else {
        return Ok(());
    };
    let directory = source.join("triggers");
    let mut status = Value::Null;
    for path in [
        directory.join(format!("{}.json", legacy.legacy_id)),
        directory.join(format!("archive/{}.json", legacy.legacy_id)),
    ] {
        match fs::read(&path) {
            Ok(bytes) => {
                status = serde_json::from_slice::<Value>(&bytes)
                    .map(|record| record["status"].clone())
                    .unwrap_or(Value::Null);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let kind = if status == "fired" {
        "edge.action_fired"
    } else {
        "edge.action_removed"
    };
    append_once(
        sessions,
        owner,
        &format!("trigger-retired:{}:{}", legacy.legacy_id, legacy.id),
        kind,
        json!({"trigger_id":legacy.id, "zsh_status":status}),
    )?;
    Ok(())
}
