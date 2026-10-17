//! Reads legacy artifacts before the destination changes.

use super::Plan;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io, path::Path};
use ws_core::ids::{SessionId, Ulid};

#[derive(Debug, Deserialize, Serialize)]
pub struct LegacySession {
    pub alias: i64,
    pub id: SessionId,
    pub meta: Value,
    pub events: Vec<LegacyLine>,
    pub messages: Vec<LegacyLine>,
    pub paste: Option<Value>,
    pub output: Option<Vec<u8>>,
    pub prompt: Option<Vec<u8>>,
    pub event_map: Vec<EventMapping>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LegacyLine {
    pub artifact: String,
    pub line: usize,
    pub raw: String,
    pub record: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EventMapping {
    pub line: usize,
    pub raw: String,
    pub seq: u64,
}

pub(super) fn snapshot(source: &Path) -> io::Result<Plan> {
    let directory = source.join("sessions");
    let counter = match optional(&directory.join("counter"))? {
        Some(bytes) => std::str::from_utf8(&bytes)
            .map_err(io::Error::other)?
            .trim()
            .parse::<i64>()
            .map_err(io::Error::other)?,
        None => 0,
    };
    let alias_floor = counter
        .checked_add(1)
        .filter(|floor| *floor > 0 && *floor < (1 << 53))
        .ok_or_else(|| io::Error::other("The zsh session counter is invalid."))?;
    let mut sessions = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Ok(alias) = entry.file_name().to_string_lossy().parse::<i64>() else {
            continue;
        };
        if alias < 1 || !entry.file_type()?.is_dir() {
            continue;
        }
        if entry.file_name() != alias.to_string().as_str() {
            return Err(io::Error::other(
                "The zsh session directory name is not canonical.",
            ));
        }
        let path = entry.path();
        let meta: Value = serde_json::from_slice(&fs::read(path.join("meta"))?)?;
        if !meta.is_object() || alias >= alias_floor {
            return Err(io::Error::other(
                "The zsh session metadata or counter is invalid.",
            ));
        }
        let events = lines(&path, "events", "event")?;
        let mut artifacts = vec![
            "inbox.inflight.paste".to_owned(),
            "inbox.inflight".to_owned(),
        ];
        let mut inflight = Vec::new();
        for file in fs::read_dir(&path)? {
            let name = file?.file_name().to_string_lossy().into_owned();
            if name.strip_prefix("inbox.inflight.").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                inflight.push(name);
            }
        }
        inflight.sort();
        artifacts.extend(inflight);
        artifacts.push("inbox".into());
        let mut messages = Vec::new();
        for artifact in artifacts {
            messages.extend(lines(&path, &artifact, "message")?);
        }
        let paste = optional(&path.join("inbox.paste"))?
            .map(|bytes| serde_json::from_slice::<Value>(&bytes))
            .transpose()?;
        if paste.as_ref().is_some_and(|paste| !paste.is_object()) {
            return Err(io::Error::other("The zsh paste record is not an object."));
        }
        sessions.push(LegacySession {
            alias,
            id: Ulid::new(),
            meta,
            events,
            messages,
            paste,
            output: optional(&path.join("output.md"))?,
            prompt: optional(&path.join("prompt"))?,
            event_map: Vec::new(),
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

fn lines(path: &Path, artifact: &str, field: &str) -> io::Result<Vec<LegacyLine>> {
    let Some(bytes) = optional(&path.join(artifact))? else {
        return Ok(Vec::new());
    };
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    text.lines()
        .enumerate()
        .map(|(index, raw)| {
            let record: Value = serde_json::from_str(raw).map_err(|error| {
                io::Error::other(format!(
                    "Invalid zsh {artifact} line {}: {error}",
                    index + 1
                ))
            })?;
            if !record[field].is_string() {
                return Err(io::Error::other(format!(
                    "The zsh {artifact} line {} requires a {field} string.",
                    index + 1
                )));
            }
            Ok(LegacyLine {
                artifact: artifact.into(),
                line: index + 1,
                raw: raw.into(),
                record,
            })
        })
        .collect()
}

fn optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
