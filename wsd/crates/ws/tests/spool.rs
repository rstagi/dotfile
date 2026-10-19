use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use ws_daemon::hooks::{Callback, spool};

mod common;
use common::{Daemon, SandboxRoot, request, ws};

#[test]
fn daemon_applies_callbacks_spooled_while_it_runs() {
    let home = SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let id = create_session(&socket);
    let callback = callback(&id, 100);
    let start = Instant::now();
    spool(&home.path().join("state"), &callback).unwrap();
    let output = ws(home.path(), &socket)
        .args([
            "wait",
            &id,
            "--for",
            "event=turn.started",
            "--timeout",
            "30",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    common::wait_for_empty_spool(&home.path().join("state"));
    if common::latency_assertions_enabled() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }
    // A transport fallback can repeat a callback that the daemon already committed.
    spool(&home.path().join("state"), &callback).unwrap();
    common::wait_for_empty_spool(&home.path().join("state"));
    let log = log(home.path(), &id);
    for event_type in ["hook.received", "turn.started"] {
        assert_eq!(
            log.iter()
                .filter(|event| event["type"] == event_type)
                .count(),
            1
        );
    }
}

#[test]
fn daemon_preserves_malformed_spool_files_and_retries_them_after_repair() {
    let home = SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let id = create_session(&socket);
    let state = home.path().join("state");
    let malformed = state.join("spool/000-malformed.json");
    fs::create_dir_all(malformed.parent().unwrap()).unwrap();
    fs::write(&malformed, "{").unwrap();
    let valid = callback(&id, 100);
    let valid_path = spool(&state, &valid).unwrap();
    let applied = ws(home.path(), &socket)
        .args([
            "wait",
            &id,
            "--for",
            "event=turn.started",
            "--timeout",
            "30",
        ])
        .output()
        .unwrap();
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(fs::read_to_string(&malformed).unwrap(), "{");
    // A repaired file must retry, even when it was present in an earlier scan.
    fs::write(&malformed, serde_json::to_vec(&valid).unwrap()).unwrap();
    common::wait_for_empty_spool(&state);
    assert!(!valid_path.exists());
    assert_eq!(
        log(home.path(), &id)
            .iter()
            .filter(|event| event["type"] == "hook.received")
            .count(),
        1
    );
}

#[test]
fn daemon_answers_ping_during_startup_replay_of_one_thousand_callbacks() {
    let home = SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    let daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let id = create_session(&socket);
    drop(daemon);
    for seq in 1..=1000 {
        let mut status = callback(&id, seq);
        status.payload = json!({});
        spool(&home.path().join("state"), &status).unwrap();
    }
    let start = Instant::now();
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let ping = ws(home.path(), &socket).arg("ping").output().unwrap();
    assert!(ping.status.success(), "{ping:?}");
    let ping_elapsed = start.elapsed();
    // Readiness must not imply that the startup spool already completed.
    assert!(
        fs::read_dir(home.path().join("state/spool"))
            .unwrap()
            .next()
            .is_some()
    );
    let live_id = create_session(&socket);
    let live_start = Instant::now();
    spool(&home.path().join("state"), &callback(&live_id, 1)).unwrap();
    let applied = ws(home.path(), &socket)
        .args([
            "wait",
            &live_id,
            "--for",
            "event=turn.started",
            "--timeout",
            "30",
        ])
        .output()
        .unwrap();
    assert!(applied.status.success(), "{applied:?}");
    let live_elapsed = live_start.elapsed();
    common::wait_for_empty_spool(&home.path().join("state"));
    let drain_elapsed = start.elapsed();
    println!("SPOOL_STARTUP ping={ping_elapsed:?} live={live_elapsed:?} drain={drain_elapsed:?}");
    if common::latency_assertions_enabled() {
        assert!(ping_elapsed < Duration::from_secs(2), "{ping_elapsed:?}");
        assert!(drain_elapsed < Duration::from_secs(60), "{drain_elapsed:?}");
        assert!(live_elapsed < Duration::from_secs(10), "{live_elapsed:?}");
    }
    let log = log(home.path(), &id);
    assert_eq!(
        log.iter()
            .filter(|event| event["type"] == "hook.received")
            .count(),
        1000
    );
    assert_eq!(
        log.iter()
            .filter(|event| event["type"] == "turn.started")
            .count(),
        0
    );
}

fn create_session(socket: &Path) -> String {
    let response = request(socket, "POST", "/v1/sessions", "{}");
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let session: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    session["id"].as_str().unwrap().into()
}

fn callback(id: &str, observed_at_us: u64) -> Callback {
    Callback {
        callback_id: ws_core::ids::Ulid::new(),
        session_id: id.parse().unwrap(),
        observed_at_us,
        state: "working".into(),
        pane: None,
        tmux_socket: None,
        tmux_path: None,
        payload: json!({"hook_event_name":"UserPromptSubmit"}),
        notify: None,
    }
}

fn log(home: &Path, id: &str) -> Vec<Value> {
    fs::read_to_string(home.join("state/sessions").join(id).join("log.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
