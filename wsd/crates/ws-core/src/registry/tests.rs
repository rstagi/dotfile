use std::collections::VecDeque;

use super::*;
use crate::{ids::Ulid, log::SessionLog};
use proptest::prelude::*;
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

#[test]
fn rebuild_replaces_the_index_from_verified_logs_and_preserves_query_results() {
    let sandbox = tempfile::tempdir().unwrap();
    let db = sandbox.path().join("registry.sqlite");
    let mut registry = Registry::open(&db, 100).unwrap();
    let sender: Ulid = "01BRZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
    let recipient: Ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAX".parse().unwrap();
    let sender_log = SessionLog::new(sandbox.path(), sender, Ulid::new());
    let recipient_log = SessionLog::new(sandbox.path(), recipient, Ulid::new());
    for (log, name) in [(&sender_log, "sender"), (&recipient_log, "recipient")] {
        registry
            .apply(
                &log.append(
                    0,
                    "session.created",
                    json!({"alias": registry.next_alias().unwrap(), "name": name}),
                )
                .unwrap(),
            )
            .unwrap();
    }
    registry
        .apply(
            &sender_log
                .append(
                    0,
                    "workspace.created",
                    json!({"workspace_id": "project/main", "path": "/sandbox/project"}),
                )
                .unwrap(),
        )
        .unwrap();
    registry
        .apply(
            &sender_log
                .append(
                    0,
                    "edge.added",
                    json!({"target": recipient, "kind": "depends_on"}),
                )
                .unwrap(),
        )
        .unwrap();
    let sent = sender_log
        .append(
            0,
            "message.sent",
            json!({"recipient": recipient, "text": "done"}),
        )
        .unwrap();
    registry.apply(&sent).unwrap();
    registry
        .apply(
            &recipient_log
                .append(0, "message.read", json!({"message_id": sent.event_id}))
                .unwrap(),
        )
        .unwrap();
    registry
        .apply(&sender_log.append(0, "session.finished", json!({})).unwrap())
        .unwrap();
    let expected = (
        registry.sessions().unwrap(),
        registry.workspaces().unwrap(),
        registry.edges().unwrap(),
        registry.messages().unwrap(),
    );
    let report = registry.rebuild(sandbox.path()).unwrap();
    assert_eq!(report.events, 7);
    assert!(report.truncated_logs.is_empty());
    assert_eq!(
        (
            registry.sessions().unwrap(),
            registry.workspaces().unwrap(),
            registry.edges().unwrap(),
            registry.messages().unwrap()
        ),
        expected
    );
    assert_eq!(registry.resolve("#100").unwrap(), sender);
    assert_eq!(registry.next_alias().unwrap(), 102);
    drop(registry);
    std::fs::remove_file(&db).unwrap();
    let mut registry = Registry::open(&db, 100).unwrap();
    registry.rebuild(sandbox.path()).unwrap();
    assert_eq!(
        (
            registry.sessions().unwrap(),
            registry.workspaces().unwrap(),
            registry.edges().unwrap(),
            registry.messages().unwrap()
        ),
        expected
    );
}

#[test]
fn rebuild_reports_crash_tails_without_changing_the_logs_and_preserves_the_index_on_failure() {
    use std::io::Write;
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let id = Ulid::new();
    let log = SessionLog::new(sandbox.path(), id, Ulid::new());
    registry
        .apply(
            &log.append(0, "session.created", json!({"alias": 1, "name": "valid"}))
                .unwrap(),
        )
        .unwrap();
    registry
        .apply(&log.append(0, "session.running", json!({})).unwrap())
        .unwrap();
    let expected = registry.sessions().unwrap();
    let path = sandbox
        .path()
        .join("sessions")
        .join(id.to_string())
        .join("log.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{crash")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let report = registry.rebuild(sandbox.path()).unwrap();
    assert_eq!(report.events, 2);
    assert_eq!(report.truncated_logs, vec![(id, 3)]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(registry.sessions().unwrap(), expected);
    let bad = log
        .append(0, "workspace.updated", json!({"workspace_id": "missing"}))
        .unwrap();
    assert!(matches!(
        registry.rebuild(sandbox.path()),
        Err(Error::InvalidEvent(_))
    ));
    assert_eq!(registry.sessions().unwrap(), expected);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"valid\"", "\"tampered\"");
    std::fs::write(&path, text).unwrap();
    assert!(matches!(
        registry.rebuild(sandbox.path()),
        Err(Error::Io(_))
    ));
    assert_eq!(registry.sessions().unwrap(), expected);
    assert!(registry.apply(&bad).is_err());
}

#[test]
fn rebuild_removes_rows_that_have_no_source_log() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 50).unwrap();
    let orphan = SessionLog::new(&sandbox.path().join("elsewhere"), Ulid::new(), Ulid::new());
    registry
        .apply(
            &orphan
                .append(0, "session.created", json!({"alias": 50}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(registry.sessions().unwrap().len(), 1);
    assert_eq!(registry.rebuild(sandbox.path()).unwrap().events, 0);
    assert!(registry.sessions().unwrap().is_empty());
    assert_eq!(registry.next_alias().unwrap(), 50);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn incremental_apply_and_rebuild_have_identical_views(
        floor in 1_i64..100_000,
        actions in prop::collection::vec((0_usize..3, 0_u8..6, 0_usize..3), 0..40),
        schedule in prop::collection::vec(0_usize..3, 1..30),
    ) {
        let sandbox = tempfile::tempdir().unwrap();
        let db = sandbox.path().join("registry.sqlite");
        let ids = [Ulid::from(3_u128), Ulid::from(1_u128), Ulid::from(2_u128)];
        let logs: Vec<_> = ids.iter().map(|id| SessionLog::new(sandbox.path(), *id, Ulid::new())).collect();
        for (actor, log) in logs.iter().enumerate() {
            log.append(0, "session.created", json!({"alias": floor + actor as i64, "name": format!("session-{actor}")})).unwrap();
            log.append(0, "workspace.created", json!({"workspace_id": format!("workspace-{actor}"), "path": format!("/sandbox/{actor}")})).unwrap();
        }
        for (index, (actor, action, target)) in actions.iter().enumerate() {
            let log = &logs[*actor];
            match action {
                0 => { log.append(0, "session.running", json!({})).unwrap(); }
                1 => { log.append(0, "session.renamed", json!({"name": format!("renamed-{actor}-{index}")})).unwrap(); }
                2 => { log.append(0, "workspace.updated", json!({"workspace_id": format!("workspace-{actor}"), "pinned": target % 2 == 0})).unwrap(); }
                3 => {
                    let edge = log.append(0, "edge.added", json!({"target": ids[*target], "kind": "depends_on"})).unwrap();
                    if target % 2 == 0 {
                        log.append(0, "edge.removed", json!({"edge_id": edge.event_id})).unwrap();
                    }
                }
                4 => {
                    let recipient = (actor + target % 2 + 1) % 3;
                    let sent = log.append(0, "message.sent", json!({"recipient": ids[recipient], "text": format!("message-{index}")})).unwrap();
                    logs[recipient].append(0, "message.delivered", json!({"message_id": sent.event_id})).unwrap();
                    logs[recipient].append(0, "message.read", json!({"message_id": sent.event_id})).unwrap();
                }
                _ => { log.append(0, "future.event", json!({"ignored": true})).unwrap(); }
            }
        }
        for (actor, log) in logs.iter().enumerate() {
            if schedule[0] == actor {
                log.append(0, "workspace.removed", json!({"workspace_id": format!("workspace-{actor}")})).unwrap();
            }
        }
        let mut queues: Vec<VecDeque<_>> = logs.iter().map(|log| log.read().unwrap().entries.into()).collect();
        let count: usize = queues.iter().map(VecDeque::len).sum();
        let mut registry = Registry::open(&db, floor).unwrap();
        for index in 0..count {
            let start = schedule[index % schedule.len()];
            let actor = (0..3).map(|offset| (start + offset) % 3).find(|actor| !queues[*actor].is_empty()).unwrap();
            let entry = queues[actor].pop_front().unwrap();
            registry.apply(&entry).unwrap();
            if index % 2 == 0 { registry.apply(&entry).unwrap(); }
        }
        let expected = query_views(&registry);
        prop_assert_eq!(registry.next_alias().unwrap(), floor + 3);
        drop(registry);
        std::fs::remove_file(&db).unwrap();
        let mut registry = Registry::open(&db, floor).unwrap();
        let report = registry.rebuild(sandbox.path()).unwrap();
        prop_assert_eq!(report.events, count);
        prop_assert_eq!(query_views(&registry), expected);
        prop_assert_eq!(registry.next_alias().unwrap(), floor + 3);
        for (actor, id) in ids.iter().enumerate() {
            prop_assert_eq!(registry.resolve(&format!("#{}", floor + actor as i64)).unwrap(), *id);
        }
    }
}

fn query_views(registry: &Registry) -> (Vec<Session>, Vec<Workspace>, Vec<Edge>, Vec<Message>) {
    (
        registry.sessions().unwrap(),
        registry.workspaces().unwrap(),
        registry.edges().unwrap(),
        registry.messages().unwrap(),
    )
}

#[test]
fn workspace_mutations_use_the_creator_log_so_replay_order_cannot_change_the_result() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let owner = Ulid::new();
    let log = SessionLog::new(sandbox.path(), owner, Ulid::new());
    registry
        .apply(
            &log.append(
                0,
                "workspace.created",
                json!({"workspace_id": "project", "pinned": false}),
            )
            .unwrap(),
        )
        .unwrap();
    let expected = registry.workspaces().unwrap();
    for event_type in ["workspace.updated", "workspace.removed"] {
        let foreign = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let entry = foreign
            .append(
                0,
                event_type,
                json!({"workspace_id": "project", "pinned": true}),
            )
            .unwrap();
        assert!(matches!(
            registry.apply(&entry),
            Err(Error::InvalidEvent(_))
        ));
        assert_eq!(registry.workspaces().unwrap(), expected);
    }
}

#[test]
fn removed_workspace_ids_keep_their_log_owner() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
    registry
        .apply(
            &log.append(0, "workspace.created", json!({"workspace_id": "project"}))
                .unwrap(),
        )
        .unwrap();
    registry
        .apply(
            &log.append(0, "workspace.removed", json!({"workspace_id": "project"}))
                .unwrap(),
        )
        .unwrap();
    assert!(registry.workspaces().unwrap().is_empty());
    let foreign = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
    let entry = foreign
        .append(0, "workspace.created", json!({"workspace_id": "project"}))
        .unwrap();
    assert!(matches!(
        registry.apply(&entry),
        Err(Error::InvalidEvent(_))
    ));
    assert!(registry.workspaces().unwrap().is_empty());
    let update = log
        .append(
            0,
            "workspace.updated",
            json!({"workspace_id": "project", "pinned": true}),
        )
        .unwrap();
    assert!(matches!(
        registry.apply(&update),
        Err(Error::InvalidEvent(_))
    ));
}

#[test]
fn rejects_malformed_session_fields_without_consuming_an_alias() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 10).unwrap();
    for body in [
        json!({}),
        json!({"alias": 0}),
        json!({"alias": -1}),
        json!({"alias": 1.5}),
        json!({"alias": "10"}),
        json!({"alias": 10, "name": []}),
        json!({"alias": 10, "name": ""}),
        json!({"alias": 10, "workspace_id": 42}),
    ] {
        let log = SessionLog::new(sandbox.path(), Ulid::new(), Ulid::new());
        let entry = log.append(0, "session.created", body.clone()).unwrap();
        assert!(
            matches!(registry.apply(&entry), Err(Error::InvalidEvent(_))),
            "{body}"
        );
        assert!(registry.sessions().unwrap().is_empty());
        assert_eq!(registry.next_alias().unwrap(), 10);
    }
}

#[test]
fn finds_a_callback_event_by_its_original_identifier_after_rebuild() {
    let sandbox = tempfile::tempdir().unwrap();
    let id = Ulid::new();
    let log = SessionLog::new(sandbox.path(), id, Ulid::new());
    log.append(0, "session.created", json!({"alias":1}))
        .unwrap();
    let captured = log
        .append(
            0,
            "hook.received",
            json!({"callback_id":"callback-1", "payload":{"tool_use_id":"tool-1"}}),
        )
        .unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    registry.rebuild(sandbox.path()).unwrap();
    assert_eq!(
        registry
            .event_for_callback(id, "hook.received", "callback-1")
            .unwrap(),
        Some(captured)
    );
    assert!(
        registry
            .event_for_callback(id, "tool.pre", "callback-1")
            .unwrap()
            .is_none()
    );
}

#[test]
fn v6_messages_deduplicate_event_ids_and_validate_sender_and_pair_sequence() {
    let sandbox = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(&sandbox.path().join("registry.sqlite"), 1).unwrap();
    let sender = Ulid::new();
    let recipient = Ulid::new();
    let log = SessionLog::new(sandbox.path(), sender, Ulid::new());
    let sent = log.append(0, "message.sent", json!({"from_session":sender,"to_session":recipient,"pair_seq":1,"body":"ready","from_alias":1})).unwrap();
    registry.apply(&sent).unwrap();
    registry.apply(&sent).unwrap();
    let messages = registry.messages().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, sent.event_id);
    assert_eq!(messages[0].sender, Some(sender));
    assert_eq!(messages[0].recipient, recipient);
    assert_eq!(messages[0].body.as_ref().unwrap()["body"], "ready");
    for patch in [
        json!({"from_session":recipient}),
        json!({"pair_seq":0}),
        json!({"pair_seq":-1}),
        json!({"pair_seq":"2"}),
        json!({"pair_seq":null}),
        json!({"body":null}),
        json!({"recipient":sender}),
    ] {
        let mut invalid = sent.clone();
        invalid.event_id = Ulid::new();
        invalid.seq = 2;
        invalid.body["pair_seq"] = json!(2);
        invalid
            .body
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(registry.apply(&invalid).is_err(), "Accepted {patch}");
        assert_eq!(registry.messages().unwrap(), messages);
    }
    let duplicate_pair = log.append(0, "message.sent", sent.body.clone()).unwrap();
    assert!(registry.apply(&duplicate_pair).is_err());
    assert_eq!(registry.messages().unwrap(), messages);
}

#[test]
fn event_queries_survive_upgrade_reopen_and_rebuild() {
    let sandbox = tempfile::tempdir().unwrap();
    let path = sandbox.path().join("registry.sqlite");
    let id = Ulid::new();
    let log = SessionLog::new(sandbox.path(), id, Ulid::new());
    log.append(0, "session.created", json!({"alias": 1}))
        .unwrap();
    let usage = log
        .append(0, "runner.usage.updated", json!({"remaining": 0.75}))
        .unwrap();
    let process_exit = log.append(0, "exit", json!({"code": 7})).unwrap();
    let hook_exit = log
        .append(0, "exit", json!({"callback_id": "exit-hook", "code": 0}))
        .unwrap();
    log.append(0, "test.noise", json!({})).unwrap();
    let mut registry = Registry::open(&path, 1).unwrap();
    registry.rebuild(sandbox.path()).unwrap();
    drop(registry);
    // Simulate the installed unversioned schema, with its existing callback index.
    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute_batch("DROP INDEX IF EXISTS applied_type;
            DROP INDEX applied_callback;
            CREATE INDEX applied_callback ON applied_events(session_id, json_extract(entry, '$.type'), json_extract(entry, '$.body.callback_id'));
            PRAGMA user_version = 0;")
        .unwrap();
    drop(legacy);
    let mut registry = Registry::open(&path, 1).unwrap();
    for rebuild in [false, true] {
        if rebuild {
            registry.rebuild(sandbox.path()).unwrap();
        }
        assert_eq!(
            registry.events_of_type("runner.usage.updated").unwrap(),
            vec![usage.clone()]
        );
        assert!(registry.events_of_type("missing").unwrap().is_empty());
        assert_eq!(
            registry
                .session_events_of_type(id, "runner.usage.updated")
                .unwrap(),
            vec![usage.clone()]
        );
        assert!(
            registry
                .session_events_of_type(Ulid::new(), "runner.usage.updated")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            registry.latest_event(id, "exit").unwrap(),
            Some(hook_exit.clone())
        );
        assert_eq!(
            registry.latest_process_exit(id).unwrap(),
            Some(process_exit.clone())
        );
        assert_eq!(registry.sessions().unwrap()[0].exit_code, Some(0));
        assert_eq!(
            registry
                .event_for_callback(id, "exit", "exit-hook")
                .unwrap(),
            Some(hook_exit.clone())
        );
    }
    assert_eq!(
        registry
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
