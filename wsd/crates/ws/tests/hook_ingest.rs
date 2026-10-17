use serde_json::{Value, json};
use std::{fs, io::Write, process::Stdio};
mod common;
use common::{Daemon, request, ws};

#[test]
fn cli_spools_every_callback_and_daemon_replays_logical_boundaries() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("wsd.sock");
    let daemon = Daemon::start(ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let created = request(&socket, "POST", "/v1/sessions", "{}");
    let created: Value = serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = created["id"].as_str().unwrap();
    drop(daemon);
    for (state, payload) in [
        (
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"p1", "turn_id":"t1"}),
        ),
        (
            "working",
            json!({"hook_event_name":"PreToolUse", "prompt_id":"p1", "turn_id":"t1", "tool_name":"Bash", "tool_use_id":"tool1"}),
        ),
        (
            "working",
            json!({"hook_event_name":"PostToolUse", "prompt_id":"p1", "turn_id":"t1", "tool_name":"Bash", "tool_use_id":"tool1", "duration_ms":50}),
        ),
        (
            "stop",
            json!({"hook_event_name":"Stop", "prompt_id":"p1", "turn_id":"t1"}),
        ),
    ] {
        let mut child = ws(home.path(), &socket)
            .env("WS_SESSION_ID", id)
            .env("TMUX_PANE", "%1")
            .env_remove("TMUX")
            .env_remove("WS_TMUX_SOCKET")
            .args(["hook", state])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let spool = home.path().join("state/spool");
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 4);
    let daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let log_path = home
        .path()
        .join("state/sessions")
        .join(id)
        .join("log.jsonl");
    let log: Vec<Value> = fs::read_to_string(&log_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let events: Vec<_> = log
        .iter()
        .filter(|entry| entry["type"] != "session.created")
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|entry| entry["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "hook.received",
            "turn.started",
            "hook.received",
            "tool.pre",
            "hook.received",
            "tool.post",
            "hook.received",
            "turn.ended"
        ]
    );
    for event in &events {
        assert_eq!(event["body"]["prompt_id"], "p1");
        assert_eq!(event["body"]["turn_id"], "t1");
        assert!(event["body"]["observed_at_us"].as_u64().unwrap() > 0);
    }
    assert_eq!(events[3]["body"]["tool_use_id"], "tool1");
    assert_eq!(events[5]["body"]["agent_duration_ms"], 50);
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 0);
    drop(daemon);
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    assert_eq!(
        fs::read_to_string(log_path).unwrap().lines().count(),
        log.len()
    );
}
