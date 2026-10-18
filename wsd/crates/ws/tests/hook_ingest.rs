use serde_json::{Value, json};
use std::{fs, io::Write, process::Stdio};
mod common;
use common::{Daemon, request, ws};

#[test]
fn cli_spools_every_callback_and_daemon_replays_logical_boundaries() {
    let home = common::SandboxRoot::new();
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

struct HookSandbox {
    home: common::SandboxRoot,
    server: String,
    socket: std::path::PathBuf,
    pane: String,
    window: String,
    id: String,
    daemon: Option<Daemon>,
}

impl HookSandbox {
    fn new() -> Self {
        Self::start(|_| {})
    }

    fn start(configure_daemon: impl FnOnce(&mut std::process::Command)) -> Self {
        let home = common::SandboxRoot::new();
        let server = home.tmux_socket();
        let socket = home.path().join("wsd.sock");
        let output = std::process::Command::new("tmux")
            .args([
                "-S",
                &server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "hooks",
                "/bin/sleep",
                "300",
            ])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let tmux = ws_daemon::tmux::Tmux::named(&server);
        let window = tmux
            .execute(&["display-message", "-p", "-t", "hooks", "#{window_id}"])
            .unwrap();
        let pane = tmux
            .execute(&["display-message", "-p", "-t", &window, "#{pane_id}"])
            .unwrap();
        let mut command = ws(home.path(), &socket);
        command.env("WS_TMUX_SOCKET", &server);
        configure_daemon(&mut command);
        let daemon = Daemon::start(command, &socket);
        let created = request(&socket, "POST", "/v1/sessions", "{}");
        let created: Value =
            serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        tmux.set_window_option(&window, "@ws_session", &id).unwrap();
        tmux.set_window_option(&window, "@ws_pane", &pane).unwrap();
        tmux.set_state(&window, "idle").unwrap();
        Self {
            home,
            server,
            socket,
            pane,
            window,
            id,
            daemon: Some(daemon),
        }
    }

    fn hook(&self, state: &str, payload: Value) -> std::time::Duration {
        let start = std::time::Instant::now();
        let mut child = ws(self.home.path(), &self.socket)
            .env("WS_SESSION_ID", &self.id)
            .env("TMUX_PANE", &self.pane)
            .env("WS_TMUX_SOCKET", &self.server)
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
        start.elapsed()
    }

    fn options(&self) -> String {
        ws_daemon::tmux::Tmux::named(&self.server)
            .execute(&[
                "display-message",
                "-p",
                "-t",
                &self.window,
                "#{@ws_state}|#{@ws_main}|#{@ws_turn}|#{@ws_sub_a}|#{@ws_sub_b}|#{@ws_unread}",
            ])
            .unwrap()
    }
}

#[test]
fn hook_window_options_match_the_zsh_hook_scenarios() {
    let sandbox = HookSandbox::new();
    let mut actual = String::new();
    for (state, payload) in [
        (
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"p1"}),
        ),
        ("waiting", json!({"hook_event_name":"PermissionRequest"})),
        ("working", json!({"hook_event_name":"PreToolUse"})),
        ("subagent-start", json!({"agent_id":"a"})),
        ("subagent-start", json!({"agent_id":"b"})),
        ("idle", json!({})),
        ("working", json!({"agent_id":"a"})),
        ("subagent-stop", json!({"agent_id":"a"})),
        ("subagent-stop", json!({"agent_id":"b"})),
        (
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "turn_id":"t2"}),
        ),
        (
            "cancel",
            json!({"hook_event_name":"Interrupt", "turn_id":"old"}),
        ),
        (
            "cancel",
            json!({"hook_event_name":"Interrupt", "turn_id":"t2"}),
        ),
    ] {
        sandbox.hook(state, payload);
        actual.push_str(&sandbox.options());
        actual.push('\n');
    }
    assert_eq!(actual, include_str!("golden/hook-states.txt"));
}

impl HookSandbox {
    fn callback(&self, state: &str, payload: Value, seq: u64) -> ws_daemon::hooks::Callback {
        ws_daemon::hooks::Callback {
            callback_id: ws_core::ids::Ulid::new(),
            session_id: self.id.parse().unwrap(),
            observed_at_us: seq,
            state: state.into(),
            pane: Some(self.pane.clone()),
            tmux_socket: Some(self.server.clone()),
            tmux_path: None,
            payload,
            notify: None,
        }
    }
    fn post(&self, state: &str, payload: Value, seq: u64) -> Value {
        let callback = self.callback(state, payload, seq);
        let response = request(
            &self.socket,
            "POST",
            "/v1/hooks/ingest",
            &serde_json::to_string(&callback).unwrap(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn log(&self) -> Vec<Value> {
        fs::read_to_string(
            self.home
                .path()
                .join("state/sessions")
                .join(&self.id)
                .join("log.jsonl"),
        )
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
    }
}

#[test]
fn hooks_apply_when_the_daemon_has_no_utf8_locale() {
    // launchd starts the daemon without LANG and without LC_* variables.
    // Without a UTF-8 locale, a tmux client prints control characters as "_".
    // TMUX in the environment also makes the tmux client use UTF-8. launchd does not set TMUX.
    let sandbox = HookSandbox::start(|daemon| {
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key == "LANG" || key.starts_with("LC_") || key.starts_with("TMUX") {
                daemon.env_remove(key.as_ref());
            }
        }
    });
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"t1"}),
        100,
    );
    assert!(
        sandbox.options().starts_with("working|working|t1|"),
        "{}",
        sandbox.options()
    );
}

#[test]
fn stale_hooks_preserve_callbacks_without_replacing_a_newer_turn() {
    let sandbox = HookSandbox::new();
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"t1"}),
        100,
    );
    sandbox.post(
        "working",
        json!({"hook_event_name":"PreToolUse", "turn_id":"t1", "tool_use_id":"tool"}),
        300,
    );
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"old"}),
        200,
    );
    assert!(sandbox.options().starts_with("working|working|t1|"));
    sandbox.post(
        "cancel",
        json!({"hook_event_name":"Interrupt", "turn_id":"old"}),
        400,
    );
    assert!(sandbox.options().starts_with("working|working|t1|"));
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"t2"}),
        500,
    );
    sandbox.post(
        "cancel",
        json!({"hook_event_name":"Interrupt", "turn_id":"t1"}),
        600,
    );
    assert!(sandbox.options().starts_with("working|working|t2|"));
    sandbox.post(
        "cancel",
        json!({"hook_event_name":"Interrupt", "turn_id":"t2"}),
        700,
    );
    assert!(sandbox.options().starts_with("idle|idle||"));
    let log = sandbox.log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "hook.received")
            .count(),
        7
    );
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "turn.cancelled")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "turn.ended")
            .count(),
        0
    );
    assert_eq!(
        ws_daemon::tmux::Tmux::named(&sandbox.server)
            .window_option(&sandbox.window, "@ws_seq")
            .unwrap()
            .as_deref(),
        Some("700")
    );
}

#[test]
fn stale_subagent_callbacks_keep_bookkeeping_and_reused_panes_keep_their_owner() {
    let sandbox = HookSandbox::new();
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "promptId":"p1"}),
        300,
    );
    sandbox.post("subagent-start", json!({"agent_id":"a"}), 100);
    assert_eq!(sandbox.options(), "working|working|p1|1||");
    sandbox.post("stop", json!({"hook_event_name":"Stop"}), 400);
    assert_eq!(sandbox.options(), "working|idle||1||");
    sandbox.post("subagent-stop", json!({"agent_id":"a"}), 500);
    assert_eq!(sandbox.options(), "idle|idle||||1");
    let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
    tmux.set_window_option(
        &sandbox.window,
        "@ws_session",
        &ws_core::ids::Ulid::new().to_string(),
    )
    .unwrap();
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"wrong"}),
        600,
    );
    assert_eq!(sandbox.options(), "idle|idle||||1");
    assert_eq!(sandbox.log().last().unwrap()["type"], "hook.received");
}

#[tokio::test]
async fn blocked_stop_preserves_the_turn_and_its_decision_across_replay() {
    use std::sync::{Arc, Mutex};
    use ws_daemon::{
        hooks::{self, StopDecision},
        sessions::Sessions,
    };
    let mut sandbox = HookSandbox::new();
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"same-turn"}),
        hooks::receipt_time_us().unwrap(),
    );
    sandbox.daemon.take();
    let state = sandbox.home.path().join("state");
    let store = Arc::new(Mutex::new(Sessions::open(&state, 1).unwrap()));
    let callback = sandbox.callback(
        "stop",
        json!({"hook_event_name":"Stop", "prompt_id":"same-turn"}),
        hooks::receipt_time_us().unwrap(),
    );
    hooks::spool(&state, &callback).unwrap();
    let reply = hooks::ingest(
        store.clone(),
        callback.clone(),
        StopDecision::Block("Continue queued work.".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        reply,
        json!({"decision":"block", "reason":"Continue queued work."})
    );
    assert!(sandbox.options().starts_with("working|working|same-turn|"));
    let reply = hooks::ingest(store.clone(), callback, StopDecision::Accept)
        .await
        .unwrap();
    assert_eq!(reply["decision"], "block");
    drop(store);
    let daemon = Daemon::start(ws(sandbox.home.path(), &sandbox.socket), &sandbox.socket);
    sandbox.daemon = Some(daemon);
    assert!(sandbox.options().starts_with("working|working|same-turn|"));
    let log = sandbox.log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "turn.continued")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "turn.ended")
            .count(),
        0
    );
    assert_eq!(fs::read_dir(state.join("spool")).unwrap().count(), 0);
}

#[test]
fn daemon_down_hooks_spool_without_tmux_and_replay_every_callback() {
    use std::os::unix::fs::PermissionsExt;
    let mut sandbox = HookSandbox::new();
    sandbox.daemon.take();
    let fake_tmux = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&fake_tmux, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let mut times = Vec::new();
    for _ in 0..100 {
        times.push(sandbox.hook("working", json!({"hook_event_name":"PreToolUse", "tool_name":"Bash", "tool_use_id":"tool"})).as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "HOOK_SPOOL samples=100 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        times[49], times[94], times[99]
    );
    if std::env::var("WS_ASSERT_LATENCY").as_deref() == Ok("1") {
        assert!(
            times[94] < 50.0,
            "The spool p95 exceeds 50 ms: {}",
            times[94]
        );
    }
    assert_eq!(
        fs::read_dir(sandbox.home.path().join("state/spool"))
            .unwrap()
            .count(),
        100
    );
    fs::remove_file(fake_tmux).unwrap();
    sandbox.daemon = Some(Daemon::start(
        ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    ));
    let log = sandbox.log();
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "hook.received")
            .count(),
        100
    );
    assert_eq!(
        log.iter()
            .filter(|entry| entry["type"] == "tool.pre")
            .count(),
        100
    );
}

#[test]
fn slow_tmux_lookup_has_a_deadline_and_preserves_the_callback() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = HookSandbox::new();
    let fake_tmux = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&fake_tmux, "#!/bin/sh\nexec /bin/sleep 5\n").unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let elapsed = sandbox.hook(
        "working",
        json!({"hook_event_name":"PreToolUse", "tool_use_id":"slow-tool"}),
    );
    assert!(
        elapsed < std::time::Duration::from_millis(400),
        "{elapsed:?}"
    );
    assert_eq!(sandbox.log().last().unwrap()["type"], "hook.received");
    assert_eq!(
        fs::read_dir(sandbox.home.path().join("state/spool"))
            .unwrap()
            .count(),
        1
    );
    assert!(common::get(&sandbox.socket, "/v1/ping").starts_with("HTTP/1.1 200"));
}

#[test]
fn a_slow_but_live_tmux_lookup_still_applies_the_callback() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = HookSandbox::new();
    let real_tmux = std::process::Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .unwrap();
    let real_tmux = String::from_utf8(real_tmux.stdout).unwrap();
    // Scheduling under load delays tmux by hundreds of milliseconds. That is not a hang.
    let fake_tmux = sandbox.home.path().join("fake-agents/tmux");
    fs::write(
        &fake_tmux,
        format!(
            "#!/bin/sh\n/bin/sleep 0.3\nexec {} \"$@\"\n",
            shell_words::quote(real_tmux.trim())
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    sandbox.post("waiting", json!({}), 100);
    assert_eq!(sandbox.options(), "waiting|||||");
}

#[test]
fn split_panes_and_untagged_unbound_windows_cannot_change_the_session_state() {
    let sandbox = HookSandbox::new();
    let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
    let pane = tmux
        .create_pane(&sandbox.window, &["/bin/sleep", "300"])
        .unwrap();
    let mut callback = sandbox.callback(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"wrong"}),
        100,
    );
    callback.pane = Some(pane);
    let response = request(
        &sandbox.socket,
        "POST",
        "/v1/hooks/ingest",
        &serde_json::to_string(&callback).unwrap(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert_eq!(sandbox.options(), "idle|||||");
    tmux.unset_window_option(&sandbox.window, "@ws_session")
        .unwrap();
    tmux.unset_window_option(&sandbox.window, "@ws_pane")
        .unwrap();
    sandbox.post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"wrong"}),
        200,
    );
    assert_eq!(sandbox.options(), "idle|||||");
}

#[test]
fn a_reused_callback_id_cannot_change_its_payload_or_receipt_time() {
    let sandbox = HookSandbox::new();
    let mut callback = sandbox.callback(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"original"}),
        100,
    );
    let first = request(
        &sandbox.socket,
        "POST",
        "/v1/hooks/ingest",
        &serde_json::to_string(&callback).unwrap(),
    );
    assert!(first.starts_with("HTTP/1.1 201"), "{first}");
    callback.observed_at_us = 200;
    callback.payload["turn_id"] = json!("changed");
    let second = request(
        &sandbox.socket,
        "POST",
        "/v1/hooks/ingest",
        &serde_json::to_string(&callback).unwrap(),
    );
    assert!(second.starts_with("HTTP/1.1 400"), "{second}");
    assert!(sandbox.options().starts_with("working|working|original|"));
    assert_eq!(sandbox.log().len(), 3);
}

#[test]
fn a_cached_delivery_reply_does_not_reach_a_reused_pane() {
    let sandbox = HookSandbox::new();
    let sent = request(
        &sandbox.socket,
        "POST",
        "/v1/messages",
        &json!({
            "target":sandbox.id, "body":"retain for the owner",
        })
        .to_string(),
    );
    assert!(sent.starts_with("HTTP/1.1 200"), "{sent}");
    let callback = sandbox.callback("stop", json!({"hook_event_name":"Stop"}), 100);
    let post = || {
        let response = request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &serde_json::to_string(&callback).unwrap(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    assert_eq!(post()["decision"], "block");
    ws_daemon::tmux::Tmux::named(&sandbox.server)
        .set_window_option(
            &sandbox.window,
            "@ws_session",
            &ws_core::ids::Ulid::new().to_string(),
        )
        .unwrap();
    assert_eq!(post(), json!({}));
    assert!(
        sandbox
            .log()
            .iter()
            .all(|entry| entry["type"] != "message.delivered")
    );
}

#[test]
fn a_numeric_legacy_ws_session_reaches_the_imported_session() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    let legacy = home.path().join(".ws/sessions");
    fs::create_dir_all(legacy.join("7")).unwrap();
    fs::write(legacy.join("counter"), "7\n").unwrap();
    fs::write(
        legacy.join("7/meta"),
        json!({"id":"7", "name":"worker", "agent":"claude", "status":"active"}).to_string(),
    )
    .unwrap();
    let _daemon = Daemon::start(ws(home.path(), &socket), &socket);
    let imported = common::get(&socket, "/v1/sessions/%237");
    let imported: Value = serde_json::from_str(imported.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = imported["id"].as_str().unwrap();
    let mut child = ws(home.path(), &socket)
        .env_remove("WS_SESSION_ID")
        .env("WS_SESSION", "7")
        .env("TMUX_PANE", "%1")
        .env_remove("TMUX")
        .env_remove("WS_TMUX_SOCKET")
        .args(["hook", "working"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"hook_event_name":"UserPromptSubmit"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let log = fs::read_to_string(
        home.path()
            .join("state/sessions")
            .join(id)
            .join("log.jsonl"),
    )
    .unwrap();
    assert!(
        log.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|entry| entry["type"] == "hook.received"),
        "{log}"
    );
}
