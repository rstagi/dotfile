use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
mod common;
use common::{Daemon, request, ws};

#[test]
fn generated_status_command_spools_quota_and_daemon_exports_it_after_restart() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude/.claude.json"),
        r#"{"oauthAccount":{"accountUuid":"custom-account"}}"#,
    )
    .unwrap();
    fs::write(
        home.path().join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"other-account"}}"#,
    )
    .unwrap();
    let daemon = Daemon::start(ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let created = request(&socket, "POST", "/v1/sessions", "{}");
    let created: Value = serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = created["id"].as_str().unwrap();
    let prepared = ws(home.path(), &socket)
        .args([
            "agent",
            "prepare",
            "--agent",
            "claude",
            "--mode",
            "headless",
            "--session",
            id,
        ])
        .output()
        .unwrap();
    assert!(prepared.status.success(), "{prepared:?}");
    let prepared: Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let file = prepared["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| {
            f["path"]
                .as_str()
                .unwrap()
                .ends_with("claude-settings.json")
        })
        .unwrap();
    let settings: Value =
        serde_json::from_slice(&fs::read(file["path"].as_str().unwrap()).unwrap()).unwrap();
    let command = shell_words::split(settings["statusLine"]["command"].as_str().unwrap()).unwrap();
    assert_eq!(command[0], env!("CARGO_BIN_EXE_ws"));
    drop(daemon);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let input = json!({"rate_limits":{"five_hour":{"used_percentage":25,"resets_at":now+3600}}});
    let mut child = ws(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read_dir(home.path().join("state/spool"))
            .unwrap()
            .count(),
        1
    );
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let path = home
        .path()
        .join("state/sessions")
        .join(id)
        .join("log.jsonl");
    let deadline = Instant::now() + Duration::from_secs(30);
    let readings = loop {
        let readings: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|e| {
                e["type"] == "runner.usage.updated"
                    && e["body"]["ws.runner.usage.source.name"] == "status-line"
            })
            .collect();
        if readings.len() >= 2 {
            break readings;
        }
        assert!(
            Instant::now() < deadline,
            "The daemon did not collect the status input."
        );
        std::thread::yield_now();
    };
    let five = readings
        .iter()
        .find(|r| r["body"]["ws.runner.usage.window"] == "five_hour")
        .unwrap();
    assert_eq!(five["body"]["ws.runner.account.id"], "custom-account");
    assert_eq!(five["body"]["ws.runner.usage.used_ratio"], 0.25);
    let weekly = readings
        .iter()
        .find(|r| r["body"]["ws.runner.usage.window"] == "weekly")
        .unwrap();
    assert!(weekly["body"].get("ws.runner.usage.used_ratio").is_none());
}

#[test]
fn supervised_claude_stream_records_account_bound_quota_at_process_exit() {
    use std::os::unix::fs::PermissionsExt;
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    let mut daemon_command = ws(home.path(), &socket);
    daemon_command.env("WS_TMUX_SOCKET", home.path().join("unused-tmux.sock"));
    fs::create_dir_all(home.path().join(".claude")).unwrap();
    fs::write(
        home.path().join(".claude/.claude.json"),
        r#"{"oauthAccount":{"accountUuid":"headless-account"}}"#,
    )
    .unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../ws-daemon/tests/fixtures/spikes/s3/claude.json"
    ))
    .unwrap();
    let mut event = fixture["headless"]["stream_events"][0].clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    event["rate_limit_info"]["unifiedWindows"]["five_hour"]["resetsAt"] = json!(now + 18000);
    event["rate_limit_info"]["unifiedWindows"]["seven_day"]["resetsAt"] = json!(now + 604800);
    fs::write(
        home.path().join("stream.jsonl"),
        format!("{{\"type\":\"system\"}}\n{event}\n"),
    )
    .unwrap();
    let program = home.path().join("fake-agents/claude");
    fs::write(
        &program,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
    printf '2.1.302\n'
    exit 0
fi
exec /bin/cat "$HOME/stream.jsonl"
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let _daemon = Daemon::start(daemon_command, &socket);
    let response = request(&socket, "POST", "/v1/sessions", &json!({
        "headless":true, "command":[program,"--output-format","stream-json","--verbose"], "cwd":home.path(),
    }).to_string());
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let created: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let path = home
        .path()
        .join("state/sessions")
        .join(created["id"].as_str().unwrap())
        .join("log.jsonl");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let entries: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let reported = entries.iter().find(|entry| {
            entry["type"] == "runner.usage.updated"
                && entry["body"]["ws.runner.usage.window"] == "weekly"
                && entry["body"]["ws.runner.usage.used_ratio"] == 0.14
        });
        if let Some(reading) = reported.filter(|_| {
            entries
                .iter()
                .any(|entry| entry["type"] == "process.exited")
        }) {
            assert_eq!(reading["body"]["ws.runner.account.id"], "headless-account");
            assert_eq!(
                reading["body"]["ws.runner.usage.source.name"],
                "stream-json"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "The daemon did not collect the headless quota."
        );
        std::thread::yield_now();
    }
}
