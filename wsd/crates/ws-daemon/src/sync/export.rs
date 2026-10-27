//! The export projection: the only log data that leaves the device.
//!
//! An entry type that is not in the allowlist is not exported. An allowlisted type exports
//! only the fields named for it. Architecture §5.1: the hub holds no secrets, only references.

use serde_json::{Map, Value};
use ws_core::log::LogEntry;
use ws_hub::sync::ExportEntry;

/// Exported types without a body: the entry keeps its event id, session id and time only.
const BARE_TYPES: &[&str] = &[
    "session.started",
    "session.running",
    "session.blocked",
    "session.finished",
    "session.failed",
    "session.reopened",
    "session.resumed",
    "session.orphaned",
    "session.failed_over",
    "done",
    "agent.waiting",
    "turn.started",
    "turn.ended",
];

/// Exported types with the string or integer fields that they keep.
const FIELD_TYPES: &[(&str, &[&str])] = &[
    ("session.created", &["alias", "name", "workspace_id", "cwd"]),
    ("session.renamed", &["name"]),
    (
        "session.repo_attached",
        &["path", "remote", "branch", "base"],
    ),
];

/// Returns the export form of an entry, or `None` when the entry stays on the device.
pub fn project(entry: &LogEntry) -> Option<ExportEntry> {
    let event_type = entry.event_type.as_str();
    let body = if BARE_TYPES.contains(&event_type) {
        Value::Object(Map::new())
    } else {
        let (_, fields) = FIELD_TYPES.iter().find(|(name, _)| *name == event_type)?;
        pick(&entry.body, fields)
    };
    Some(ExportEntry {
        event_id: entry.event_id.to_string(),
        session_id: entry.session_id.to_string(),
        seq: entry.seq,
        ts_ms: entry.ts_ms,
        event_type: entry.event_type.clone(),
        body,
    })
}

fn pick(body: &Value, fields: &[&str]) -> Value {
    let mut picked = Map::new();
    for field in fields {
        match &body[*field] {
            Value::String(text) if *field == "remote" => {
                picked.insert((*field).into(), Value::String(strip_credentials(text)));
            }
            value @ (Value::String(_) | Value::Number(_)) => {
                picked.insert((*field).into(), value.clone());
            }
            _ => {}
        }
    }
    Value::Object(picked)
}

/// Removes `user:token@` from a URL remote.
fn strip_credentials(remote: &str) -> String {
    match remote.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest
                .split_once('/')
                .map_or((rest, None), |(a, p)| (a, Some(p)));
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            match path {
                Some(path) => format!("{scheme}://{host}/{path}"),
                None => format!("{scheme}://{host}"),
            }
        }
        None => remote.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use ws_core::ids::Ulid;

    const SECRET: &str = "sk-live-SECRET-1234";

    fn entry(event_type: &str, body: Value) -> LogEntry {
        LogEntry {
            event_id: Ulid::new(),
            session_id: Ulid::new(),
            epoch: 0,
            seq: 7,
            prev_hash: String::new(),
            ts_ms: 99,
            event_type: event_type.into(),
            body,
            device_id: Ulid::nil(),
            sig: String::new(),
        }
    }

    #[test]
    fn no_export_form_contains_a_secret() {
        let secret_body = json!({
            "payload": {"tool_input": {"command": format!("curl -H {SECRET}")}, "tool_response": SECRET},
            "body": SECRET, "text": SECRET, "argv": [SECRET], "command": SECRET, "output": SECRET,
            "reason": SECRET, "callback_id": SECRET,
        });
        for event_type in [
            "hook.received",
            "message.sent",
            "tool.result",
            "exit",
            "agent.started",
            "session.created",
            "session.running",
            "session.finished",
            "session.renamed",
            "session.repo_attached",
            "agent.waiting",
            "turn.started",
            "turn.ended",
            "done",
        ] {
            if let Some(exported) = project(&entry(event_type, secret_body.clone())) {
                assert!(
                    !serde_json::to_string(&exported).unwrap().contains(SECRET),
                    "{event_type}"
                );
            }
        }
    }

    #[test]
    fn unlisted_types_are_not_exported() {
        for event_type in [
            "hook.received",
            "message.sent",
            "tool.result",
            "exit",
            "workspace.created",
        ] {
            assert!(
                project(&entry(event_type, json!({"x": 1}))).is_none(),
                "{event_type}"
            );
        }
    }

    #[test]
    fn activity_events_keep_only_id_session_and_time() {
        let source = entry("turn.ended", json!({"summary": "text"}));
        let exported = project(&source).unwrap();
        assert_eq!(exported.event_id, source.event_id.to_string());
        assert_eq!(exported.session_id, source.session_id.to_string());
        assert_eq!((exported.seq, exported.ts_ms), (7, 99));
        assert_eq!(exported.body, json!({}));
    }

    #[test]
    fn created_keeps_allowlisted_fields_only() {
        let exported = project(&entry(
            "session.created",
            json!({"alias": 3, "name": "n", "cwd": "/w", "workspace_id": "w1", "argv": ["x"], "env": {"K": SECRET}}),
        ))
        .unwrap();
        assert_eq!(
            exported.body,
            json!({"alias": 3, "name": "n", "cwd": "/w", "workspace_id": "w1"})
        );
    }

    #[test]
    fn repo_remote_loses_credentials() {
        let exported = project(&entry(
            "session.repo_attached",
            json!({"path": "/r", "remote": format!("https://user:{SECRET}@github.com/a/b.git"), "branch": "m"}),
        ))
        .unwrap();
        assert_eq!(exported.body["remote"], "https://github.com/a/b.git");
        assert_eq!(exported.body["branch"], "m");
    }
}
