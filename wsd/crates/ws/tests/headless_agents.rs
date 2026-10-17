mod common;

use serde_json::Value;
use std::fs;
use ws_daemon::agents::Prepared;

#[test]
fn prepare_prints_the_shared_contract_and_materializes_private_files_without_a_daemon() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("absent.sock");
    let id = "01K7Q0K5M00000000000000001";
    for agent in ["claude", "codex", "grok"] {
        for resume in [None, Some("resume-123")] {
            let mut command = common::ws(home.path(), &socket);
            command.args([
                "agent",
                "prepare",
                "--agent",
                agent,
                "--mode",
                "headless",
                "--session",
                id,
                "--format",
                "json",
            ]);
            if let Some(resume) = resume {
                command.args(["--resume", resume]);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let prepared: Prepared = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(prepared.program, agent);
            assert_eq!(prepared.env["WS_SESSION_ID"], id);
            assert_eq!(prepared.env["WS_SESSION"], id);
            for file in &prepared.files {
                assert!(file.path.starts_with(home.path()));
                assert_eq!(fs::read(&file.path).unwrap(), file.contents);
                assert_eq!(
                    fs::metadata(&file.path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            if agent == "claude" {
                assert_eq!(prepared.argv[0], "-p");
                let index = prepared
                    .argv
                    .iter()
                    .position(|arg| arg == "--mcp-config")
                    .unwrap();
                let mcp: Value =
                    serde_json::from_slice(&fs::read(&prepared.argv[index + 1]).unwrap()).unwrap();
                assert_eq!(mcp["mcpServers"]["ws"]["command"], env!("CARGO_BIN_EXE_ws"));
                assert_eq!(mcp["mcpServers"]["ws"]["args"], serde_json::json!(["mcp"]));
                assert_eq!(mcp["mcpServers"]["ws"]["env"]["WS_SESSION_ID"], id);
                let settings = &prepared.argv[prepared
                    .argv
                    .iter()
                    .position(|v| v == "--settings")
                    .unwrap()
                    + 1];
                let hooks: Value = serde_json::from_slice(&fs::read(settings).unwrap()).unwrap();
                let hook = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap();
                assert_eq!(
                    shell_words::split(hook).unwrap(),
                    [env!("CARGO_BIN_EXE_ws"), "hook", "stop"]
                );
            } else if agent == "codex" {
                assert_eq!(prepared.argv[0], "exec");
                assert!(
                    prepared
                        .argv
                        .windows(2)
                        .any(|pair| pair == ["-c", "mcp_servers.ws.args=[\"mcp\"]"])
                );
                assert!(
                    prepared
                        .argv
                        .iter()
                        .any(|arg| arg == &format!("mcp_servers.ws.env.WS_SESSION_ID=\"{id}\""))
                );
            } else {
                assert!(home.path().join(".grok/hooks/ws.json").is_file());
            }
            if resume.is_some() {
                assert!(prepared.argv.windows(2).any(|pair| pair
                    == [
                        if agent == "codex" {
                            "resume"
                        } else {
                            "--resume"
                        },
                        "resume-123"
                    ]));
            }
            assert!(!socket.exists());
        }
    }
}

#[test]
fn shell_output_preserves_quoting_and_the_claude_print_prompt_order() {
    use std::process::Command;
    let home = tempfile::Builder::new()
        .prefix("ws ' headless ")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = home.path().join("absent.sock");
    let output = common::ws(home.path(), &socket)
        .args([
            "agent",
            "prepare",
            "--agent",
            "claude",
            "--mode",
            "headless",
            "--session",
            "01K7Q0K5M00000000000000001",
            "--format",
            "sh",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let script = r#"
        eval "$1"
        print_flag=$1
        shift
        exec /usr/bin/python3 -c "$PROBE" "$WS_AGENT_PROGRAM" "$WS_AGENT_FILES" "$print_flag" "$PROMPT" "$@" --allowedTools Read Bash
    "#;
    for shell in ["/bin/sh", "zsh"] {
        let captured = Command::new(shell).args(["-c", script, "probe"])
            .arg(std::str::from_utf8(&output.stdout).unwrap())
            .env("PROMPT", "Return 'READY'\nThen stop. $(false)")
            .env("PROBE", "import json,os,sys; print(json.dumps(dict(argv=sys.argv[1:], id=os.environ['WS_SESSION_ID'], legacy=os.environ['WS_SESSION'])))")
            .output().unwrap();
        assert!(captured.status.success(), "{captured:?}");
        let captured: Value = serde_json::from_slice(&captured.stdout).unwrap();
        assert_eq!(captured["id"], "01K7Q0K5M00000000000000001");
        assert_eq!(captured["id"], captured["legacy"]);
        let args = captured["argv"].as_array().unwrap();
        assert_eq!(args[0], "claude");
        let files = shell_words::split(args[1].as_str().unwrap()).unwrap();
        assert_eq!(files.len(), 2);
        assert!(std::path::Path::new(&files[0]).is_file());
        assert_eq!(args[2], "-p");
        assert_eq!(args[3], "Return 'READY'\nThen stop. $(false)");
        assert_eq!(&args[args.len() - 3..], ["--allowedTools", "Read", "Bash"]);
    }
}

#[test]
fn fresh_and_resumed_headless_agents_emit_ordered_events_for_the_supplied_session() {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("wsd.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let fake = r#"#!/usr/bin/python3
import json, os, re, subprocess, sys
agent = os.path.basename(sys.argv[0])
args = sys.argv[1:]
assert args[0] == ('-p' if agent == 'claude' else 'exec'), args
if agent == 'claude':
    with open(args[args.index('--settings') + 1]) as source:
        hooks = json.load(source)['hooks']
    commands = {event: rules[0]['hooks'][0]['command'] for event, rules in hooks.items()}
else:
    commands = {}
    for arg in args:
        if arg.startswith('hooks.'):
            event = arg.split('=', 1)[0][6:]
            commands[event] = json.loads(re.search(r'command=("(?:[^"\\]|\\.)*")', arg).group(1))
key = 'prompt_id' if agent == 'claude' else 'turn_id'
turn = 'resumed-turn' if 'resume-123' in args else 'fresh-turn'
for event in ['UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'Stop']:
    payload = {'hook_event_name': event, key: turn, 'session_id': 'native-agent-id'}
    if 'Tool' in event:
        payload.update(tool_name='Bash', tool_use_id='tool-123')
    subprocess.run(commands[event], shell=True, input=json.dumps(payload), text=True, capture_output=True, check=True, timeout=30)
print(json.dumps({'argv': args, 'id': os.environ['WS_SESSION_ID']}))
"#;
    for agent in ["claude", "codex"] {
        let program = home.path().join("fake-agents").join(agent);
        fs::write(&program, fake).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        for resume in [false, true] {
            let response = common::request(&socket, "POST", "/v1/sessions", "{}");
            assert!(response.starts_with("HTTP/1.1 201"), "{response}");
            let created: Value =
                serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
            let id = created["id"].as_str().unwrap();
            let mut command = common::ws(home.path(), &socket);
            command.args([
                "agent",
                "prepare",
                "--agent",
                agent,
                "--mode",
                "headless",
                "--session",
                id,
                "--format",
                "json",
            ]);
            if resume {
                command.args(["--resume", "resume-123"]);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let prepared: Prepared = serde_json::from_slice(&output.stdout).unwrap();
            let result = Command::new(&program)
                .args(&prepared.argv)
                .envs(&prepared.env)
                .env("WS_SESSION", "wrong-legacy-id")
                .env("HOME", home.path())
                .env("WS_HOME", home.path().join(".ws"))
                .env("WS_STATE", home.path().join("state"))
                .env("WS_SOCK", &socket)
                .env("WS_TMUX_SOCKET", "headless-must-not-contact-tmux")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
            let captured: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(captured["id"], id);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let events = loop {
                let events = session_events(home.path(), id);
                if events.len() == 4 {
                    break events;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "The agent hooks did not reach the session log: {events:?}"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!(
                events
                    .iter()
                    .map(|e| e["type"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["turn.started", "tool.pre", "tool.post", "turn.ended"]
            );
            for event in &events {
                assert_eq!(event["session_id"], id);
                assert_eq!(event["body"]["state_accepted"], true);
                assert!(event["body"]["window_id"].is_null());
                assert_eq!(
                    event["body"][if agent == "claude" {
                        "prompt_id"
                    } else {
                        "turn_id"
                    }],
                    if resume { "resumed-turn" } else { "fresh-turn" }
                );
            }
            assert_eq!(events[1]["body"]["tool_use_id"], "tool-123");
            assert_eq!(events[2]["body"]["tool_use_id"], "tool-123");
            assert_eq!(events[0]["body"]["window_state"], "working");
            assert_eq!(events[3]["body"]["window_state"], "idle");
        }
    }
}

#[test]
fn headless_sequence_and_turn_guards_survive_restart_and_spool_replay_per_session() {
    use serde_json::json;
    use ws_core::ids::Ulid;
    use ws_daemon::hooks::{self, Callback};
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("wsd.sock");
    let daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let create = || {
        let response = common::request(&socket, "POST", "/v1/sessions", "{}");
        let session: Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        session["id"].as_str().unwrap().to_owned()
    };
    let first = create();
    let second = create();
    let callback = |id: &str, state: &str, event: &str, turn: &str, seq| Callback {
        callback_id: Ulid::new(),
        session_id: id.parse().unwrap(),
        observed_at_us: seq,
        state: state.into(),
        pane: None,
        tmux_socket: Some("must-not-use-tmux".into()),
        tmux_path: None,
        payload: json!({"hook_event_name": event, "turn_id": turn}),
        notify: None,
    };
    let post = |callback: &Callback| {
        let response = common::request(
            &socket,
            "POST",
            "/v1/hooks/ingest",
            &serde_json::to_string(callback).unwrap(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    };
    let start = callback(&first, "working", "UserPromptSubmit", "first-turn", 300);
    post(&start);
    post(&start);
    post(&callback(
        &second,
        "working",
        "UserPromptSubmit",
        "second-turn",
        100,
    ));
    // A callback for another session can have a lower sequence number.
    assert_eq!(
        session_events(home.path(), &second)[0]["body"]["state_accepted"],
        true
    );
    drop(daemon);
    let replay = callback(&first, "working", "PreToolUse", "first-turn", 400);
    hooks::spool(&home.path().join("state"), &replay).unwrap();
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    post(&callback(&first, "stop", "Stop", "first-turn", 350));
    post(&callback(&first, "cancel", "Interrupt", "wrong-turn", 500));
    post(&callback(
        &second,
        "cancel",
        "Interrupt",
        "second-turn",
        101,
    ));
    post(&callback(&first, "cancel", "Interrupt", "first-turn", 600));
    let first_events = session_events(home.path(), &first);
    assert_eq!(
        first_events
            .iter()
            .map(|e| e["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["turn.started", "tool.pre", "turn.cancelled"]
    );
    assert_eq!(first_events[1]["body"]["state_accepted"], true);
    assert_eq!(first_events[2]["body"]["window_state"], "idle");
    let second_events = session_events(home.path(), &second);
    assert_eq!(
        second_events
            .iter()
            .map(|e| e["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["turn.started", "turn.cancelled"]
    );
    assert_eq!(second_events[1]["body"]["state_accepted"], true);
    assert_eq!(
        fs::read_dir(home.path().join("state/spool"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn a_headless_blocked_stop_keeps_its_turn_and_decision_after_restart() {
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use ws_core::ids::Ulid;
    use ws_daemon::{
        hooks::{self, Callback, StopDecision},
        sessions::{CreateSession, Sessions},
    };
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let state = home.path().join("state");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let id = sessions.create(CreateSession::default()).unwrap().id;
    let store = Arc::new(Mutex::new(sessions));
    let callback = |event: &str, state: &str, seq| Callback {
        callback_id: Ulid::new(),
        session_id: id,
        observed_at_us: seq,
        state: state.into(),
        pane: None,
        tmux_socket: None,
        tmux_path: None,
        payload: json!({"hook_event_name":event, "turn_id":"same-turn"}),
        notify: None,
    };
    hooks::ingest(
        store.clone(),
        callback("UserPromptSubmit", "working", 10),
        StopDecision::Accept,
    )
    .await
    .unwrap();
    let blocked = callback("Stop", "stop", 20);
    let reply = hooks::ingest(
        store.clone(),
        blocked.clone(),
        StopDecision::Block("Continue queued work.".into()),
    )
    .await
    .unwrap();
    assert_eq!(reply["decision"], "block");
    drop(store);
    let store = Arc::new(Mutex::new(Sessions::open(&state, 1).unwrap()));
    let reply = hooks::ingest(store.clone(), blocked, StopDecision::Accept)
        .await
        .unwrap();
    assert_eq!(reply["decision"], "block");
    hooks::ingest(
        store,
        callback("Interrupt", "cancel", 30),
        StopDecision::Accept,
    )
    .await
    .unwrap();
    let events = session_events(home.path(), &id.to_string());
    assert_eq!(
        events
            .iter()
            .map(|entry| entry["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["turn.started", "turn.continued", "turn.cancelled"]
    );
    assert_eq!(events[1]["body"]["window_state"], "working");
    assert_eq!(events[2]["body"]["state_accepted"], true);
    assert_eq!(events[2]["body"]["window_state"], "idle");
}

fn session_events(home: &std::path::Path, id: &str) -> Vec<Value> {
    fs::read_to_string(home.join("state/sessions").join(id).join("log.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| {
            entry["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("turn.") || kind.starts_with("tool."))
        })
        .collect()
}
