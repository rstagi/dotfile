use super::*;
use crate::{ids::Ulid, log::SessionLog};
use serde_json::json;

#[test]
fn sessions_survive_reopening_and_aliases_start_at_the_configured_floor() {
    let sandbox = tempfile::tempdir().unwrap();
    let db = sandbox.path().join("registry.sqlite");
    let session = Ulid::new();
    let device = Ulid::new();
    let log = SessionLog::new(sandbox.path(), session, device);
    let mut registry = Registry::open(&db, 100).unwrap();
    assert_eq!(registry.next_alias().unwrap(), 100);
    let entry = log
        .append(0, "session.created", json!({"alias": 100, "name": "build"}))
        .unwrap();
    registry.apply(&entry).unwrap();
    drop(registry);
    let registry = Registry::open(&db, 100).unwrap();
    let sessions = registry.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session);
    assert_eq!(sessions[0].alias, 100);
    assert_eq!(sessions[0].name.as_deref(), Some("build"));
    assert_eq!(sessions[0].state, "created");
    assert_eq!(sessions[0].device_id, device);
    assert_eq!(registry.next_alias().unwrap(), 101);
}

#[test]
fn resolves_alias_name_and_ulid_and_rejects_ambiguous_prefixes() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 20).unwrap();
    let first: Ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
    let second: Ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAZ".parse().unwrap();
    for (session, name) in [(first, "build"), (second, "review")] {
        let log = SessionLog::new(sandbox.path(), session, Ulid::new());
        let alias = registry.next_alias().unwrap();
        registry
            .apply(
                &log.append(0, "session.created", json!({"alias": alias, "name": name}))
                    .unwrap(),
            )
            .unwrap();
    }
    assert_eq!(registry.resolve("#20").unwrap(), first);
    assert_eq!(registry.resolve("#21").unwrap(), second);
    assert_eq!(registry.resolve("build").unwrap(), first);
    assert_eq!(
        registry.resolve(&first.to_string().to_lowercase()).unwrap(),
        first
    );
    assert!(matches!(
        registry.resolve("01ARZ3"),
        Err(Error::Ambiguous(_))
    ));
    for input in ["", "#", "#0", "#22", "absent", "%", "_"] {
        assert!(
            matches!(registry.resolve(input), Err(Error::NotFound(_))),
            "{input}"
        );
    }
    let third: Ulid = "01BRZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
    let log = SessionLog::new(sandbox.path(), third, Ulid::new());
    registry
        .apply(
            &log.append(0, "session.created", json!({"alias": 22, "name": "build"}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(registry.resolve("01B").unwrap(), third);
    assert!(matches!(
        registry.resolve("build"),
        Err(Error::Ambiguous(_))
    ));
}

#[test]
fn projects_lifecycle_and_name_changes_once_and_rolls_back_invalid_events() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let id = Ulid::new();
    let log = SessionLog::new(sandbox.path(), id, Ulid::new());
    let created = log
        .append(0, "session.created", json!({"alias": 1, "name": "before"}))
        .unwrap();
    registry.apply(&created).unwrap();
    let renamed = log
        .append(0, "session.renamed", json!({"name": "after"}))
        .unwrap();
    registry.apply(&renamed).unwrap();
    for state in ["running", "blocked", "running", "finished", "reopened"] {
        let event = log
            .append(0, &format!("session.{state}"), json!({}))
            .unwrap();
        registry.apply(&event).unwrap();
    }
    registry.apply(&created).unwrap();
    registry.apply(&renamed).unwrap();
    let sessions = registry.sessions().unwrap();
    assert_eq!(sessions[0].name.as_deref(), Some("after"));
    assert_eq!(sessions[0].state, "reopened");
    assert!(matches!(
        registry.resolve("before"),
        Err(Error::NotFound(_))
    ));
    assert_eq!(registry.resolve("after").unwrap(), id);
    assert_eq!(registry.next_alias().unwrap(), 2);
    let mut conflicting = created.clone();
    conflicting.body["name"] = json!("conflicting");
    assert!(matches!(
        registry.apply(&conflicting),
        Err(Error::InvalidEvent(_))
    ));
    assert_eq!(registry.sessions().unwrap(), sessions);
    let mut skipped = log.append(0, "session.failed", json!({})).unwrap();
    skipped.seq += 1;
    assert!(matches!(
        registry.apply(&skipped),
        Err(Error::InvalidEvent(_))
    ));
    assert_eq!(registry.sessions().unwrap(), sessions);
    let other_log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
    let duplicate_alias = other_log
        .append(
            0,
            "session.created",
            json!({"alias": 1, "name": "collision"}),
        )
        .unwrap();
    assert!(registry.apply(&duplicate_alias).is_err());
    assert_eq!(registry.sessions().unwrap(), sessions);
}

#[test]
fn projects_workspace_changes_and_removal_from_the_log() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
    let created = log
        .append(
            0,
            "workspace.created",
            json!({"workspace_id": "app/main", "path": "/sandbox/app", "pinned": false}),
        )
        .unwrap();
    registry.apply(&created).unwrap();
    let workspaces = registry.workspaces().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, "app/main");
    assert_eq!(workspaces[0].body["path"], "/sandbox/app");
    let updated = log
        .append(
            0,
            "workspace.updated",
            json!({"workspace_id": "app/main", "pinned": true}),
        )
        .unwrap();
    registry.apply(&updated).unwrap();
    let workspaces = registry.workspaces().unwrap();
    assert_eq!(workspaces[0].body["pinned"], true);
    assert_eq!(workspaces[0].body["path"], "/sandbox/app");
    let removed = log
        .append(0, "workspace.removed", json!({"workspace_id": "app/main"}))
        .unwrap();
    registry.apply(&removed).unwrap();
    assert!(registry.workspaces().unwrap().is_empty());
    registry.apply(&created).unwrap();
    assert!(registry.workspaces().unwrap().is_empty());
}

#[test]
fn projects_edges_and_removes_only_the_named_edge() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let source = Ulid::new();
    let target = Ulid::new();
    let log = SessionLog::new(sandbox.path(), source, Ulid::new());
    let mut edges = Vec::new();
    for kind in ["depends_on", "block_on", "parent"] {
        let entry = log
            .append(0, "edge.added", json!({"target": target, "kind": kind}))
            .unwrap();
        registry.apply(&entry).unwrap();
        edges.push(entry);
    }
    let projected = registry.edges().unwrap();
    assert_eq!(projected.len(), 3);
    assert!(
        projected
            .iter()
            .all(|edge| edge.source == source && edge.target == target)
    );
    assert!(projected.iter().any(|edge| edge.kind == "block_on"));
    let removed = log
        .append(0, "edge.removed", json!({"edge_id": edges[1].event_id}))
        .unwrap();
    registry.apply(&removed).unwrap();
    assert_eq!(registry.edges().unwrap().len(), 2);
    assert!(
        registry
            .edges()
            .unwrap()
            .iter()
            .all(|edge| edge.id != edges[1].event_id)
    );
    registry.apply(&edges[1]).unwrap();
    assert_eq!(registry.edges().unwrap().len(), 2);
}

#[test]
fn message_receipts_survive_projection_before_the_sender_log() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let sender = Ulid::new();
    let recipient = Ulid::new();
    let sender_log = SessionLog::new(sandbox.path(), sender, Ulid::new());
    let recipient_log = SessionLog::new(sandbox.path(), recipient, Ulid::new());
    let sent = sender_log
        .append(
            0,
            "message.sent",
            json!({"recipient": recipient, "text": "ready", "pair_seq": 1}),
        )
        .unwrap();
    let delivered = recipient_log
        .append(0, "message.delivered", json!({"message_id": sent.event_id}))
        .unwrap();
    let read = recipient_log
        .append(0, "message.read", json!({"message_id": sent.event_id}))
        .unwrap();
    registry.apply(&delivered).unwrap();
    registry.apply(&read).unwrap();
    registry.apply(&sent).unwrap();
    registry.apply(&sent).unwrap();
    let messages = registry.messages().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, sent.event_id);
    assert_eq!(messages[0].sender, Some(sender));
    assert_eq!(messages[0].recipient, recipient);
    assert_eq!(messages[0].body.as_ref().unwrap()["text"], "ready");
    assert_eq!(messages[0].sent_at_ms, Some(sent.ts_ms as i64));
    assert_eq!(messages[0].delivered_at_ms, Some(delivered.ts_ms as i64));
    assert_eq!(messages[0].read_at_ms, Some(read.ts_ms as i64));
    let other_log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
    let wrong_recipient = other_log
        .append(0, "message.read", json!({"message_id": sent.event_id}))
        .unwrap();
    assert!(matches!(
        registry.apply(&wrong_recipient),
        Err(Error::InvalidEvent(_))
    ));
    assert_eq!(registry.messages().unwrap(), messages);
}
