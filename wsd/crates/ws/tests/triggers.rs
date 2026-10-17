#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn command(home: &Path, socket: &Path) -> Command {
    let mut command = common::ws(home, socket);
    for key in ["WS_SESSION", "WS_SESSION_ID", "TMUX", "TMUX_PANE"] {
        command.env_remove(key);
    }
    command
}

fn run(home: &Path, socket: &Path, args: &[&str]) -> Output {
    command(home, socket).args(args).output().unwrap()
}

fn api(socket: &Path, method: &str, path: &str, body: Value) -> Value {
    let response = common::request(socket, method, path, &body.to_string());
    assert!(response.starts_with("HTTP/1.1 20"), "{response}");
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[test]
fn on_and_triggers_preserve_zsh_listing_removal_and_queued_send_contracts() {
    // Golden scenarios: ws/tests/triggers*.test.sh and trigger-edges.test.sh.
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let source = api(&socket, "POST", "/v1/sessions", json!({"name":"source"}));
    let owner = api(&socket, "POST", "/v1/sessions", json!({"name":"owner"}));
    let registration = command(home.path(), &socket)
        .env("WS_SESSION_ID", owner["id"].as_str().unwrap())
        .args([
            "on",
            "source",
            "done",
            "--send-to",
            "me",
            "--template",
            "result {id}: {output}",
        ])
        .output()
        .unwrap();
    assert!(registration.status.success(), "{registration:?}");
    let tid = String::from_utf8(registration.stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert!(tid.parse::<ws_core::ids::Ulid>().is_ok());
    let listing = run(home.path(), &socket, &["triggers"]);
    assert!(listing.status.success(), "{listing:?}");
    assert_eq!(
        String::from_utf8(listing.stdout).unwrap(),
        format!(
            "{tid}\t{}\tdone\tsend\t{}\tresult {{id}}: {{output}}\n",
            source["id"].as_str().unwrap(),
            owner["id"].as_str().unwrap()
        )
    );
    let listing = run(home.path(), &socket, &["triggers", "--json"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&listing.stdout).unwrap()[0]["id"],
        tid
    );
    assert!(
        run(home.path(), &socket, &["on", "--rm", &tid])
            .status
            .success()
    );
    assert_eq!(
        run(home.path(), &socket, &["on", "--rm", &tid])
            .status
            .code(),
        Some(1)
    );
    assert!(run(home.path(), &socket, &["triggers"]).stdout.is_empty());
    api(
        &socket,
        "POST",
        &format!("/v1/sessions/{}/done", source["id"].as_str().unwrap()),
        json!({"output":"literal {id} $value\n"}),
    );
    let immediate = run(
        home.path(),
        &socket,
        &["on", "source", "done", "--send-to", "owner", "-"],
    );
    assert!(immediate.status.success(), "{immediate:?}");
    let inbox = api(
        &socket,
        "GET",
        &format!("/v1/inbox?session_id={}", owner["id"].as_str().unwrap()),
        Value::Null,
    );
    assert_eq!(inbox[0]["body"], "-");
    let human = api(
        &socket,
        "GET",
        &format!(
            "/v1/sessions/{}",
            inbox[0]["from_session"].as_str().unwrap()
        ),
        Value::Null,
    );
    assert_eq!(human["body"]["human_device_id"], human["device_id"]);
    for arguments in [
        vec!["on", "source", "wrong", "--send-to", "owner", "hello"],
        vec!["on", "source", "done", "--send-to", "owner", "{output}"],
        vec!["on", "source,,owner", "done", "--send-to", "owner", "text"],
        vec!["on", "source", "done", "--spawn", "unknown", "prompt"],
    ] {
        assert!(!run(home.path(), &socket, &arguments).status.success());
    }
}

#[test]
fn restart_fires_a_durable_satisfaction_and_never_repeats_the_message() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let source = api(&socket, "POST", "/v1/sessions", json!({}));
    let owner = api(&socket, "POST", "/v1/sessions", json!({}));
    let trigger = api(
        &socket,
        "POST",
        "/v1/triggers",
        json!({"sources":[source["id"]], "event":"done", "target":owner["id"], "template":"recovered {output}", "parent_session_id":owner["id"]}),
    );
    drop(daemon);
    // Simulate a crash after the source append but before action evaluation.
    let state = home.path().join("state");
    let source_id = source["id"].as_str().unwrap().parse().unwrap();
    let device = source["device_id"].as_str().unwrap().parse().unwrap();
    let path = state
        .join("sessions")
        .join(source["id"].as_str().unwrap())
        .join("output.md");
    std::fs::write(&path, "result\n").unwrap();
    ws_core::log::SessionLog::new(&state, source_id, device)
        .append(0, "done", json!({"output":path}))
        .unwrap();
    for _ in 0..2 {
        std::fs::remove_file(state.join("registry.sqlite")).unwrap();
        let daemon = common::Daemon::start(command(home.path(), &socket), &socket);
        assert_eq!(api(&socket, "GET", "/v1/triggers", Value::Null), json!([]));
        assert_eq!(api(&socket, "GET", "/v1/edges", Value::Null), json!([]));
        let inbox = api(
            &socket,
            "GET",
            &format!("/v1/inbox?session_id={}", owner["id"].as_str().unwrap()),
            Value::Null,
        );
        assert_eq!(inbox.as_array().unwrap().len(), 1);
        assert_eq!(inbox[0]["body"], "recovered result\n");
        drop(daemon);
    }
    let entries = ws_core::log::SessionLog::new(
        &state,
        owner["id"].as_str().unwrap().parse().unwrap(),
        device,
    )
    .read()
    .unwrap()
    .entries;
    let message = entries
        .iter()
        .find(|entry| entry.event_type == "message.sent")
        .unwrap();
    assert_eq!(message.body["trigger_id"], trigger["id"]);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.event_type == "message.sent")
            .count(),
        1
    );
}
