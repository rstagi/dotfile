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
