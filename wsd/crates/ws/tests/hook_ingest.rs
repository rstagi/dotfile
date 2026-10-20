use serde_json::{Value, json};
use std::{fs, io::Write, process::Stdio};
mod common;
use common::{Daemon, request, ws};

struct HookProcess(std::process::Child);

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
    common::wait_for_empty_spool(&home.path().join("state"));
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
        let mut daemon = Daemon::spawn(command);
        daemon.wait_ready_for(&socket, std::time::Duration::from_secs(30));
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
fn missing_pane_exit_hooks_record_exit_fire_triggers_and_replay_once() {
    for (spooled, server_present) in [(false, true), (true, true), (false, false), (true, false)] {
        let mut sandbox = HookSandbox::new();
        let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
        tmux.execute(&["new-window", "-d", "-t", "hooks:", "/bin/sleep", "300"])
            .unwrap();
        let registered = request(
            &sandbox.socket,
            "POST",
            "/v1/triggers",
            &json!({"sources":[sandbox.id], "event":"exit", "target":sandbox.id,
                "parent_session_id":sandbox.id, "template":"The agent exited."})
            .to_string(),
        );
        assert!(registered.starts_with("HTTP/1.1 201"), "{registered}");
        let callback = sandbox.callback(
            "exit",
            json!({"hook_event_name":"SessionEnd", "exit_code":7}),
            ws_daemon::hooks::receipt_time_us().unwrap(),
        );
        tmux.execute(&["kill-window", "-t", &sandbox.window])
            .unwrap();
        if !server_present {
            tmux.execute(&["kill-server"]).unwrap();
        }
        if spooled {
            sandbox.daemon.take();
            ws_daemon::hooks::spool(&sandbox.home.path().join("state"), &callback).unwrap();
            sandbox.daemon = Some(Daemon::start(
                ws(sandbox.home.path(), &sandbox.socket),
                &sandbox.socket,
            ));
            common::wait_for_empty_spool(&sandbox.home.path().join("state"));
            assert_eq!(
                fs::read_dir(sandbox.home.path().join("state/spool"))
                    .unwrap()
                    .count(),
                0
            );
        } else {
            let reply = request(
                &sandbox.socket,
                "POST",
                "/v1/hooks/ingest",
                &serde_json::to_string(&callback).unwrap(),
            );
            assert!(reply.starts_with("HTTP/1.1 201"), "{reply}");
        }
        let retry = request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &serde_json::to_string(&callback).unwrap(),
        );
        assert!(retry.starts_with("HTTP/1.1 201"), "{retry}");
        let log = sandbox.log();
        let exits: Vec<_> = log.iter().filter(|entry| entry["type"] == "exit").collect();
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0]["body"]["code"], 7);
        assert_eq!(
            log.iter()
                .filter(|entry| entry["type"] == "message.sent")
                .count(),
            1
        );
        let waited = ws(sandbox.home.path(), &sandbox.socket)
            .args(["wait", &sandbox.id, "--for", "exit", "--timeout", "0"])
            .output()
            .unwrap();
        assert!(waited.status.success(), "{waited:?}");
    }
}

#[test]
fn spooled_hooks_of_a_removed_pane_replay_once_and_leave_the_spool() {
    for server_present in [true, false] {
        let mut sandbox = HookSandbox::new();
        let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
        tmux.execute(&["new-window", "-d", "-t", "hooks:", "/bin/sleep", "300"])
            .unwrap();
        let callback = sandbox.callback(
            "working",
            json!({"hook_event_name":"PreToolUse"}),
            ws_daemon::hooks::receipt_time_us().unwrap(),
        );
        tmux.execute(&["kill-window", "-t", &sandbox.window])
            .unwrap();
        if !server_present {
            tmux.execute(&["kill-server"]).unwrap();
        }
        sandbox.daemon.take();
        ws_daemon::hooks::spool(&sandbox.home.path().join("state"), &callback).unwrap();
        sandbox.daemon = Some(Daemon::start(
            ws(sandbox.home.path(), &sandbox.socket),
            &sandbox.socket,
        ));
        // The daemon drains the spool after the socket opens.
        let spool = sandbox.home.path().join("state/spool");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while fs::read_dir(&spool).unwrap().count() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(
            fs::read_dir(&spool).unwrap().count(),
            0,
            "server_present={server_present}"
        );
        let received = sandbox
            .log()
            .iter()
            .filter(|entry| entry["type"] == "hook.received")
            .count();
        assert_eq!(received, 1, "server_present={server_present}");
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
        sandbox.post(state, payload, ws_daemon::hooks::receipt_time_us().unwrap());
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
fn replayed_working_callback_preserves_newer_idle_state_and_records_its_event() {
    for headless in [false, true] {
        for restart in [false, true] {
            let mut sandbox = HookSandbox::new();
            let mut working = sandbox.callback(
                "working",
                json!({"hook_event_name":"UserPromptSubmit", "turn_id":"old"}),
                100,
            );
            if headless {
                working.pane = None;
                working.tmux_socket = None;
            }
            let mut idle = working.clone();
            idle.callback_id = ws_core::ids::Ulid::new();
            idle.observed_at_us = 300;
            idle.state = "stop".into();
            idle.payload = json!({"hook_event_name":"Stop"});
            for callback in [&working, &idle] {
                let response = request(
                    &sandbox.socket,
                    "POST",
                    "/v1/hooks/ingest",
                    &serde_json::to_string(callback).unwrap(),
                );
                assert!(response.starts_with("HTTP/1.1 201"), "{response}");
            }
            working.callback_id = ws_core::ids::Ulid::new();
            working.observed_at_us = 200;
            if restart {
                sandbox.daemon.take();
            }
            let state = sandbox.home.path().join("state");
            ws_daemon::hooks::spool(&state, &working).unwrap();
            if restart {
                sandbox.daemon = Some(Daemon::start(
                    ws(sandbox.home.path(), &sandbox.socket),
                    &sandbox.socket,
                ));
            }
            common::wait_for_empty_spool(&state);
            if !headless {
                assert!(sandbox.options().starts_with("idle|idle||"));
            }
            let log = sandbox.log();
            let events: Vec<_> = log
                .iter()
                .filter(|event| event["body"]["callback_id"] == working.callback_id.to_string())
                .collect();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0]["type"], "hook.received");
            assert_eq!(events[1]["type"], "turn.started");
            assert_eq!(events[1]["body"]["state_accepted"], false);
            assert_eq!(events[1]["body"]["window_state"], "idle");
        }
    }
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
    common::wait_for_empty_spool(&state);
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
    if common::latency_assertions_enabled() {
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
    common::wait_for_empty_spool(&sandbox.home.path().join("state"));
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
    if common::latency_assertions_enabled() {
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "{elapsed:?}"
        );
    }
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
fn a_sandbox_can_extend_the_hook_tmux_deadline() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = HookSandbox::start(|command| {
        command.env("WS_TEST_HOOK_TMUX_DEADLINE_MS", "30000");
    });
    let real_tmux = std::process::Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .unwrap();
    let real_tmux = String::from_utf8(real_tmux.stdout).unwrap();
    let fake_tmux = sandbox.home.path().join("fake-agents/tmux");
    // Inject a command delay beyond the production deadline. This is not a readiness wait.
    fs::write(
        &fake_tmux,
        format!(
            "#!/bin/sh\ncase \" $* \" in *' show-options '*) /bin/sleep 3 ;; esac\nexec {} \"$@\"\n",
            shell_words::quote(real_tmux.trim())
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    sandbox.post("waiting", json!({}), 100);
    assert_eq!(sandbox.options(), "waiting|||||");
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

#[test]
fn a_cancelled_parent_redelivers_a_committed_batch_once() {
    let sandbox = HookSandbox::new();
    sandbox.queue("retain after parent cancellation");
    let callback = sandbox.callback(
        "stop",
        json!({"hook_event_name":"Stop", "stop_hook_active":false}),
        100,
    );
    let reply = sandbox.ingest(&callback);
    assert_eq!(reply["decision"], "block");
    sandbox.confirm(&callback);
    let retry = sandbox.post(
        "stop",
        json!({"hook_event_name":"Stop", "stop_hook_active":false}),
        200,
    );
    assert_eq!(retry["reason"], reply["reason"]);
    let final_reply = sandbox.post(
        "stop",
        json!({"hook_event_name":"Stop", "stop_hook_active":true}),
        300,
    );
    assert_eq!(final_reply, json!({}));
    assert!(sandbox.options().starts_with("idle|idle|"));
    assert_eq!(
        sandbox
            .log()
            .iter()
            .filter(|e| e["type"] == "message.delivered")
            .count(),
        1
    );
    assert_eq!(
        sandbox
            .log()
            .iter()
            .filter(|e| e["type"] == "message.read")
            .count(),
        1
    );
}

impl HookSandbox {
    fn queue(&self, text: &str) {
        let response = request(
            &self.socket,
            "POST",
            "/v1/messages",
            &json!({"target":self.id,"body":text}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }
    fn ingest(&self, callback: &ws_daemon::hooks::Callback) -> Value {
        let response = request(
            &self.socket,
            "POST",
            "/v1/hooks/ingest",
            &serde_json::to_string(callback).unwrap(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn confirm(&self, callback: &ws_daemon::hooks::Callback) {
        let response = request(
            &self.socket,
            "POST",
            "/v1/hooks/delivered",
            &json!({"session_id":self.id,"callback_id":callback.callback_id}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }
}

#[test]
fn the_stop_cli_budget_fits_every_generated_parent_timeout() {
    use ws_daemon::agents::{AgentSession, Mode, prepare};
    let home = common::SandboxRoot::new();
    let session = AgentSession {
        agent_program: None,
        state: home.path().join("state"),
        parent_session_id: None,
        project: "sandbox".into(),
        id: ws_core::ids::Ulid::new().to_string(),
        home: home.path().into(),
        grok_home: None,
        codex_home: None,
        claude_config_dir: None,
        ws_home: home.path().join(".ws"),
        ws_bin: env!("CARGO_BIN_EXE_ws").into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for agent in ["claude", "grok"] {
        let prepared = prepare(agent, Mode::Interactive, &session, None).unwrap();
        let settings: Value = serde_json::from_slice(&prepared.files[0].contents).unwrap();
        for event in if agent == "claude" {
            vec!["Stop", "SessionEnd"]
        } else {
            vec!["Stop"]
        } {
            let timeout = settings["hooks"][event][0]["hooks"][0]["timeout"]
                .as_u64()
                .unwrap();
            assert_eq!(timeout, 30);
            assert!(ws_daemon::hooks::STOP_CLI_BUDGET < std::time::Duration::from_secs(timeout));
        }
    }
}

#[test]
fn parent_deadline_after_stdout_preserves_a_committed_receipt() {
    parent_deadline_after_stdout(false);
}

#[test]
fn parent_deadline_after_stdout_preserves_a_spooled_receipt() {
    parent_deadline_after_stdout(true);
}

fn parent_deadline_after_stdout(spooled: bool) {
    use std::{
        io::{BufRead, BufReader, Read},
        os::unix::net::UnixListener,
        time::{Duration, Instant},
    };
    let sandbox = HookSandbox::new();
    sandbox.queue("arrives after the parent discards stdout");
    let proxy_path = sandbox.home.path().join("proxy.sock");
    let listener = UnixListener::bind(&proxy_path).unwrap();
    let socket = sandbox.socket.clone();
    let proxy = std::thread::spawn(move || {
        for path in ["/v1/hooks/ingest", "/v1/hooks/delivered"] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(60)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut header = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                header.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            assert!(header.starts_with(&format!("POST {path} ")), "{header}");
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            if spooled && path == "/v1/hooks/delivered" {
                // Keep the receipt in the spool until the production CLI budget expires.
                assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
            } else {
                let reply = request(&socket, "POST", path, std::str::from_utf8(&body).unwrap());
                if spooled && path == "/v1/hooks/ingest" {
                    // Spend part of the total budget before stdout, then withhold confirmation.
                    tokio::runtime::Runtime::new().unwrap().block_on(async {
                        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                    });
                }
                stream.write_all(reply.as_bytes()).unwrap();
            }
        }
    });
    let parent_deadline = Instant::now() + Duration::from_secs(30);
    let mut child = HookProcess(
        ws(sandbox.home.path(), &proxy_path)
            .env_remove("WS_HOOK_TRANSPORT_DEADLINE_MS")
            .env("WS_SESSION_ID", &sandbox.id)
            .env("TMUX_PANE", &sandbox.pane)
            .env("WS_TMUX_SOCKET", &sandbox.server)
            .args(["hook", "stop"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"hook_event_name\":\"Stop\",\"stop_hook_active\":false}")
        .unwrap();
    let mut output = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut output)
        .unwrap();
    let discarded: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(discarded["decision"], "block");
    // The parent cancels its decision at the generated deadline, even after stdout succeeds.
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        tokio::time::sleep_until(tokio::time::Instant::from_std(parent_deadline)).await;
    });
    let _ = child.kill();
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "The CLI exceeded its parent deadline: {status}"
    );
    proxy.join().unwrap();
    assert!(sandbox.options().starts_with("working|working|"));
    // The live drain can confirm a spooled receipt before the parent deadline.
    let confirmed = ws(sandbox.home.path(), &sandbox.socket)
        .args([
            "wait",
            &sandbox.id,
            "--for",
            "event=hook.confirmed",
            "--timeout",
            "30",
        ])
        .output()
        .unwrap();
    assert!(confirmed.status.success(), "{confirmed:?}");
    let recovered = sandbox.cli_stop(false);
    assert_eq!(recovered, discarded);
    assert_eq!(sandbox.cli_stop(true), json!({}));
    assert!(sandbox.options().starts_with("idle|idle|"));
    for event in ["message.delivered", "message.read"] {
        assert_eq!(
            sandbox.log().iter().filter(|e| e["type"] == event).count(),
            1,
            "{event}"
        );
    }
}

impl HookSandbox {
    fn cli_stop(&self, active: bool) -> Value {
        let mut child = ws(self.home.path(), &self.socket)
            .env_remove("WS_HOOK_TRANSPORT_DEADLINE_MS")
            .env("WS_SESSION_ID", &self.id)
            .env("TMUX_PANE", &self.pane)
            .env("WS_TMUX_SOCKET", &self.server)
            .args(["hook", "stop"])
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
                json!({"hook_event_name":"Stop", "stop_hook_active":active})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn recovery_preserves_a_turn_that_starts_after_the_window_snapshot() {
    recovery_preserves_newer_turn(false);
}

#[test]
fn recovery_retries_a_timed_out_snapshot_and_preserves_a_newer_turn() {
    recovery_preserves_newer_turn(true);
}

fn recovery_preserves_newer_turn(timeout_snapshot: bool) {
    use std::{
        io::{BufRead, BufReader},
        os::unix::{fs::PermissionsExt, net::UnixListener},
        time::Duration,
    };
    let gate_home = tempfile::tempdir_in("/tmp").unwrap();
    let gate_path = gate_home.path().join("gate.sock");
    let gate = UnixListener::bind(&gate_path).unwrap();
    let slow_marker = gate_home.path().join("slow-snapshot");
    let real_tmux = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("tmux"))
        .find(|path| path.is_file())
        .unwrap();
    let wrapper = gate_home.path().join("tmux");
    fs::write(
        &wrapper,
        format!(
            r#"#!/usr/bin/python3
import os, socket, subprocess, sys, time
if 'show-options' in sys.argv[1:]:
    try:
        os.rename({slow:?}, {slow:?} + '.used')
        # Fault injection: exceed the sandbox deadline once to exercise recovery retries.
        time.sleep(31)
    except FileNotFoundError:
        pass
if any('display-message -p released' in arg for arg in sys.argv[1:]):
    s = socket.socket(socket.AF_UNIX)
    s.connect({gate:?})
    s.sendall(b'ready\n')
    s.recv(1)
    result = subprocess.run([{tmux:?}] + sys.argv[1:], capture_output=True)
    os.write(1, result.stdout)
    os.write(2, result.stderr)
    s.sendall(b'finished\n')
    sys.exit(result.returncode)
os.execv({tmux:?}, [{tmux:?}] + sys.argv[1:])
"#,
            gate = gate_path.to_str().unwrap(),
            slow = slow_marker.to_str().unwrap(),
            tmux = real_tmux.to_str().unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let sandbox = HookSandbox::start(|command| {
        let path = command
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .unwrap()
            .1
            .unwrap()
            .to_string_lossy()
            .into_owned();
        command.env("PATH", format!("{}:{path}", gate_home.path().display()));
        command.env("WS_TEST_HOOK_TMUX_DEADLINE_MS", "30000");
    });
    sandbox.queue("keep the newer turn");
    assert_eq!(sandbox.unconfirmed_stop()["decision"], "block");
    if timeout_snapshot {
        fs::write(&slow_marker, "").unwrap();
    }
    // The connection proves that recovery read the snapshot and reached the window guard.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(gate.accept().map(|(stream, _)| stream));
    });
    let mut control = receiver
        .recv_timeout(Duration::from_secs(90))
        .expect("Recovery did not reach its window guard.")
        .unwrap();
    assert_eq!(
        slow_marker.with_extension("used").exists(),
        timeout_snapshot
    );
    control
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let mut reader = BufReader::new(control.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
    tmux.set_window_option(
        &sandbox.window,
        "@ws_seq",
        &ws_daemon::hooks::receipt_time_us().unwrap().to_string(),
    )
    .unwrap();
    tmux.set_window_option(&sandbox.window, "@ws_turn", "newer-turn")
        .unwrap();
    control.write_all(b"x").unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "finished\n");
    assert!(sandbox.options().starts_with("working|working|newer-turn|"));
    assert_eq!(
        sandbox.cli_stop(false)["reason"],
        "[ws #2]\nkeep the newer turn"
    );
    assert_eq!(sandbox.cli_stop(true), json!({}));
    assert!(sandbox.options().starts_with("idle|idle|"));
    assert_eq!(
        sandbox
            .log()
            .iter()
            .filter(|e| e["type"] == "message.delivered")
            .count(),
        1
    );
}

impl HookSandbox {
    fn unconfirmed_stop(&self) -> Value {
        let callback = self.callback(
            "stop",
            json!({"hook_event_name":"Stop", "stop_hook_active":false}),
            ws_daemon::hooks::receipt_time_us().unwrap(),
        );
        self.ingest(&callback)
    }
}

impl std::ops::Deref for HookProcess {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for HookProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for HookProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
