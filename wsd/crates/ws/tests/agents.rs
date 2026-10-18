mod common;

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Sandbox {
    home: common::SandboxRoot,
    socket: PathBuf,
    server: String,
    listener: UnixListener,
    _daemon: common::Daemon,
}

#[test]
fn an_exit_self_trigger_reopens_after_the_old_launch_retires() {
    for code in [0, 7] {
        let sandbox = Sandbox::new();
        let repo = sandbox.repo();
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    "feature",
                    "--detach"
                ])
                .status
                .success()
        );
        let (owner, mut old_agent) = sandbox.capture();
        let id = owner["id"].as_str().unwrap();
        let old_window = sandbox.window(id);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["on", id, "exit", "--send-to", "me", "continue after exit"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let end_hook = common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id":id, "callback_id":ws_core::ids::Ulid::new(),
                "observed_at_us":ws_daemon::hooks::receipt_time_us().unwrap(), "state":"exit",
                "pane":owner["pane"], "tmux_socket":sandbox.server,
                "payload":{"hook_event_name":"SessionEnd", "exit_code":0}
            })
            .to_string(),
        );
        assert!(end_hook.starts_with("HTTP/1.1 201"), "{end_hook}");
        assert!(sandbox.events(id, "message.sent").is_empty());
        old_agent
            .write_all(if code == 0 { b"0" } else { b"7" })
            .unwrap();
        let waited =
            sandbox.output(&["wait", id, "--for", "event=message.sent", "--timeout", "30"]);
        assert!(
            waited.status.success(),
            "The message wait failed: {waited:?}"
        );
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        assert_ne!(sandbox.window(id), old_window);
        let (reopened, _running) = sandbox.capture();
        assert_eq!(reopened["id"], id);
        assert_eq!(sandbox.events(id, "message.sent").len(), 1);
    }
}

#[test]
fn failed_trigger_launch_releases_its_slot_and_keeps_diagnostics() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (owner, _running) = sandbox.capture();
    let owner_id = owner["id"].as_str().unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let source_id = source["id"].as_str().unwrap();
    let output = sandbox
        .command()
        .env("WS_SESSION_ID", owner_id)
        .args(["on", source_id, "done", "--spawn", "claude", "continue"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let tid = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let real_tmux = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("tmux"))
        .find(|path| path.is_file())
        .unwrap();
    let wrapper = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&wrapper, format!("#!/bin/sh\ncase \" $* \" in *' new-window '*|*' new-session '*) echo 'synthetic launch failure' >&2; exit 1;; esac\nexec {} \"$@\"\n", shell_words::quote(&real_tmux.to_string_lossy()))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let done = format!("/v1/sessions/{source_id}/done");
    for _ in 0..2 {
        assert!(common::request(&sandbox.socket, "POST", &done, "{}").starts_with("HTTP/1.1 200"));
    }
    assert_eq!(sandbox.json(&["triggers", "--json"]), json!([]));
    let sessions = sandbox.json(&["sessions", "--json"]);
    let children: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|session| session["body"]["trigger_id"] == tid)
        .collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["state"], "failed");
    assert!(
        !fs::read_to_string(sandbox.slots())
            .unwrap()
            .contains(children[0]["id"].as_str().unwrap())
    );
    let failures = sandbox.events(owner_id, "edge.action_failed");
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].body["error"]
            .as_str()
            .unwrap()
            .contains("synthetic launch failure")
    );
}

#[test]
fn trigger_spawn_preserves_a_literal_prompt_and_closes_the_child_on_done() {
    // Golden scenarios: ws/tests/triggers-actions.test.sh.
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (owner, _running) = sandbox.capture();
    let owner_id = owner["id"].as_str().unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let source_id = source["id"].as_str().unwrap();
    assert!(
        common::request(
            &sandbox.socket,
            "POST",
            &format!("/v1/sessions/{source_id}/done"),
            r#"{"output":"literal {id} $value\n"}"#
        )
        .starts_with("HTTP/1.1 200")
    );
    for (agent, legacy) in [
        ("claude", false),
        ("codex", true),
        ("grok", false),
        ("claude", true),
    ] {
        let suffix = if agent == "grok" {
            " large".repeat(6000)
        } else {
            String::new()
        };
        let prompt = format!(
            "Follow \"quoted\" $literal\nsource {{id}}: {{output}}; $(touch never-execute-ws-prompt){suffix}"
        );
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", owner_id)
            .args(if legacy && agent == "claude" {
                vec!["on", source_id, "done", "--spawn", &prompt]
            } else if legacy {
                vec![
                    "on", source_id, "done", "--spawn", &prompt, "--agent", agent,
                ]
            } else {
                vec!["on", source_id, "done", "--spawn", agent, &prompt]
            })
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let tid = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let (child, _connection) = sandbox.capture();
        let id = child["id"].as_str().unwrap();
        assert_ne!(id, owner_id);
        assert_eq!(child["agent"], agent);
        let argv = child["argv"].as_array().unwrap();
        let expected = format!(
            "Follow \"quoted\" $literal\nsource {source_id}: literal {{id}} $value\n; $(touch never-execute-ws-prompt){suffix}"
        );
        assert!(
            argv.iter().any(|argument| argument == &expected),
            "{argv:?}"
        );
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(session["body"]["spawned"], true);
        assert_eq!(session["body"]["trigger_id"], tid);
        assert_eq!(session["body"]["parent_session_id"], owner_id);
        let window = sandbox.window(id);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .arg("done")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            !sandbox
                .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
                .lines()
                .any(|line| line == window)
        );
        assert!(!fs::read_to_string(sandbox.slots()).unwrap().contains(id));
    }
    let output = sandbox
        .command()
        .current_dir(sandbox.worktree())
        .args(["on", source_id, "done", "--spawn", "codex"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let (child, _running) = sandbox.capture();
    let child_id = child["id"].as_str().unwrap();
    let session = sandbox.json(&["show", child_id, "--json"]);
    let parent = sandbox.json(&[
        "show",
        session["body"]["parent_session_id"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(parent["body"]["human_device_id"], parent["device_id"]);
    assert_eq!(session["workspace_id"], "api/feature");
    assert!(child["argv"].as_array().unwrap().iter().any(|argument| {
        argument
            .as_str()
            .is_some_and(|text| text.contains(source_id) && text.contains("literal {id} $value"))
    }));
    assert!(
        sandbox
            .command()
            .env("WS_SESSION_ID", child_id)
            .arg("done")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!sandbox.worktree().join("never-execute-ws-prompt").exists());
}

#[test]
fn a_finished_live_window_cannot_consume_the_reopened_sessions_message() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach",
            ])
            .status
            .success()
    );
    let (initial, _running) = sandbox.capture();
    let id = initial["id"].as_str().unwrap();
    let old_window = sandbox.window(id);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &old_window, "#{pane_id}"]);
    let stop = || {
        common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id": id, "callback_id": ws_core::ids::Ulid::new(),
                "observed_at_us": ws_daemon::hooks::receipt_time_us().unwrap(),
                "state": "stop", "pane": pane, "tmux_socket": sandbox.server,
                "payload": {"hook_event_name":"Stop", "session_id":"retained-conversation"}
            })
            .to_string(),
        )
    };
    assert!(stop().starts_with("HTTP/1.1 201"));
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{id}/stop"),
        "{}",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        sandbox
            .output(&["send", id, "only for the new window"])
            .status
            .code(),
        Some(75)
    );
    let (resumed, _resumed) = sandbox.capture();
    assert_eq!(resumed["id"], id);
    let new_window = sandbox.tmux(&[
        "display-message",
        "-p",
        "-t",
        resumed["pane"].as_str().unwrap(),
        "#{window_id}",
    ]);
    assert_ne!(new_window, old_window);
    let response = stop();
    let reply: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(reply.get("decision").is_none(), "{reply}");
    assert!(sandbox.events(id, "hook.delivery").is_empty());
    assert_eq!(sandbox.window(id), new_window);
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
}

#[test]
fn a_message_reopens_a_finished_agent_before_delivery() {
    for agent in ["claude", "codex"] {
        let sandbox = Sandbox::new();
        let repo = sandbox.repo();
        let output = sandbox.output(&[
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "feature",
            "--agent",
            agent,
            "--detach",
        ]);
        assert!(output.status.success(), "{output:?}");
        let (first, mut connection) = sandbox.capture();
        let id = first["id"].as_str().unwrap();
        let old_window = sandbox.window(id);
        let pane = sandbox.tmux(&["display-message", "-p", "-t", &old_window, "#{pane_id}"]);
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id": id, "callback_id": ws_core::ids::Ulid::new(),
                "observed_at_us": ws_daemon::hooks::receipt_time_us().unwrap(),
                "state": "stop", "pane": pane, "tmux_socket": sandbox.server,
                "payload": {"hook_event_name":"Stop", "session_id":format!("resume-{agent}")}
            })
            .to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        connection.write_all(b"0").unwrap();
        sandbox.wait_for(|| sandbox.json(&["show", id, "--json"])["state"] == "finished");
        assert!(!fs::read_to_string(sandbox.slots()).unwrap().contains(id));

        let output = if agent == "codex" {
            sandbox.output(&["send", id, "continue the task", "--force"])
        } else {
            sandbox.output(&["send", id, "continue the task"])
        };
        assert_eq!(output.status.code(), Some(75), "{output:?}");
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        let (resumed, mut running) = sandbox.capture();
        assert_eq!(resumed["id"], id);
        assert_ne!(sandbox.window(id), old_window);
        assert!(
            resumed["argv"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|args| args
                    == [
                        json!(if agent == "codex" {
                            "resume"
                        } else {
                            "--resume"
                        }),
                        json!(format!("resume-{agent}")),
                    ])
        );
        assert!(sandbox.events(id, "message.delivered").is_empty());
        let reopened = sandbox.events(id, "session.reopened");
        assert_eq!(reopened.len(), 1);
        let sent = sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|session| sandbox.events(session["id"].as_str().unwrap(), "message.sent"))
            .find(|event| event.body["to_session"] == id)
            .unwrap();
        assert!(sent.ts_ms <= reopened[0].ts_ms);
        running.write_all(b"d").unwrap();
        let mut reader = std::io::BufReader::new(running.try_clone().unwrap());
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        assert_eq!(
            line.trim_end(),
            "[ws #2]\ncontinue the task".escape_debug().to_string()
        );
        assert!(sandbox.events(id, "message.delivered").is_empty());
        running.write_all(b"a").unwrap();
        sandbox.wait_for(|| sandbox.events(id, "message.delivered").len() == 1);
        let delivered = sandbox.events(id, "message.delivered");
        assert_eq!(delivered[0].body["message_id"], sent.event_id.to_string());
        let entries = sandbox.log(id);
        let reopen = entries
            .iter()
            .position(|e| e.event_type == "session.reopened")
            .unwrap();
        let deliver = entries
            .iter()
            .position(|e| e.event_type == "message.delivered")
            .unwrap();
        assert!(reopen < deliver);
    }
}

#[test]
fn workspace_agents_receive_hooks_identity_slots_and_resume_ids() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let mut agents = vec![];
    for (i, agent) in ["claude", "codex", "grok"].into_iter().enumerate() {
        let output = if i == 0 {
            sandbox.output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--agent",
                agent,
                "--detach",
            ])
        } else {
            sandbox.output(&["add", "api/feature", "--agent", agent, "--detach"])
        };
        assert!(output.status.success(), "{output:?}");
        let (capture, _running) = sandbox.capture();
        assert_eq!(capture["agent"], agent);
        assert_eq!(capture["shell"], "ready");
        assert_eq!(capture["id"], capture["legacy_id"]);
        assert_eq!(
            capture["cwd"],
            sandbox.worktree().canonicalize().unwrap().to_str().unwrap()
        );
        let sessions = sandbox.json(&["sessions", "--json"]);
        let session = sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == capture["id"])
            .unwrap();
        let slot_file = sandbox.slots();
        let rows = fs::read_to_string(&slot_file).unwrap();
        assert!(rows.contains(&format!(
            "{}\t{agent}\t\t{}\n",
            i + 1,
            session["id"].as_str().unwrap()
        )));
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        if agent == "claude" {
            let settings =
                Path::new(&args[args.iter().position(|v| v == "--settings").unwrap() + 1]);
            let hooks: Value = serde_json::from_slice(&fs::read(settings).unwrap()).unwrap();
            let hook = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap();
            assert_eq!(
                shell_words::split(hook).unwrap(),
                [env!("CARGO_BIN_EXE_ws"), "hook", "stop"]
            );
            assert!(settings.starts_with(sandbox.home.path()));
        } else if agent == "codex" {
            assert!(args.iter().any(|s| s == "--dangerously-bypass-hook-trust"));
            assert!(args.iter().any(|s| s.starts_with("hooks.Stop=")));
        } else {
            assert!(sandbox.home.path().join(".grok/hooks/ws.json").is_file());
        }
        let id = session["id"].as_str().unwrap();
        assert_mcp_registration(&args, agent, id);
        let rows = rows.replace(
            &format!("\t{agent}\t\t{id}"),
            &format!("\t{agent}\tresume-{agent}\t{id}"),
        );
        fs::write(slot_file, rows).unwrap();
        let window = sandbox.window(id);
        sandbox.tmux(&["kill-window", "-t", &window]);
        let output = sandbox.output(&["open", id, "--detach"]);
        assert!(output.status.success(), "{output:?}");
        let (resumed, running) = sandbox.capture();
        agents.push(running);
        assert_eq!(resumed["id"], id);
        let resumed_args: Vec<String> = serde_json::from_value(resumed["argv"].clone()).unwrap();
        assert_mcp_registration(&resumed_args, agent, id);
        let args = resumed["argv"].as_array().unwrap();
        assert!(args.windows(2).any(|p| p
            == [
                json!(if agent == "codex" {
                    "resume"
                } else {
                    "--resume"
                }),
                json!(format!("resume-{agent}"))
            ]));
    }
}

#[test]
fn named_agent_commands_run_in_the_current_tab_and_preserve_failed_slots() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (_initial, _running) = sandbox.capture();
    for (agent, exit_code) in [("claude", 0), ("codex", 7), ("grok", 0)] {
        let window = sandbox.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            "api/feature:",
            "-c",
            sandbox.worktree().to_str().unwrap(),
            "/bin/cat",
        ]);
        let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
        let mut command = sandbox.command();
        command
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent, "Review this", "--model", "test-model"]);
        let child = command.spawn().unwrap();
        let (capture, mut connection) = sandbox.capture();
        assert_eq!(capture["workspace"], "api/feature");
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        assert_eq!(
            &args[args.len() - 3..],
            ["Review this", "--model", "test-model"]
        );
        let id = capture["id"].as_str().unwrap();
        assert_eq!(sandbox.window(id), window);
        let output = sandbox
            .command()
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("this tab already runs a ws session")
        );
        connection
            .write_all(exit_code.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(exit_code), "{output:?}");
        assert_eq!(
            sandbox.tmux(&["show-options", "-wqv", "-t", &window, "@ws_slot"]),
            ""
        );
        let rows = fs::read_to_string(sandbox.slots()).unwrap();
        assert_eq!(rows.contains(id), exit_code != 0);
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(session["exit_code"], exit_code);
        let response = common::request(
            &sandbox.socket,
            "POST",
            &format!("/v1/sessions/{id}/wait"),
            r#"{"for":"exit","timeout":0}"#,
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(
            session["state"],
            if exit_code == 0 {
                "finished"
            } else {
                "running"
            }
        );
        sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    }
}

#[test]
fn legacy_agent_and_end_wrappers_run_an_imported_zsh_session() {
    let mut pane = String::new();
    let sandbox = Sandbox::with_setup("", |home, server| {
        let repo = home.join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        let wt = home.join("worktrees/api/feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        let ids = Command::new("tmux")
            .args([
                "-S",
                server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "legacy",
            ])
            .args(["-P", "-F", "#{window_id}\t#{pane_id}", "/bin/cat"])
            .output()
            .unwrap();
        assert!(ids.status.success(), "{ids:?}");
        let ids = String::from_utf8(ids.stdout).unwrap();
        let (window, legacy_pane) = ids.trim_end().split_once('\t').unwrap();
        pane = legacy_pane.into();
        for (key, value) in [("@ws_session", "7"), ("@ws_pane", legacy_pane)] {
            let status = Command::new("tmux")
                .args(["-S", server, "set-option", "-w", "-t", window, key, value])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let legacy = home.join(".ws/sessions/7");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(home.join(".ws/sessions/counter"), "7").unwrap();
        fs::write(
            legacy.join("meta"),
            json!({"agent": "claude", "workspace": "api/feature", "window": window}).to_string(),
        )
        .unwrap();
        fs::write(legacy.join("prompt"), "hello legacy").unwrap();
        let git_dir = git(&wt, &["rev-parse", "--absolute-git-dir"]);
        fs::write(
            Path::new(&git_dir).join("ws-sessions"),
            "1\tclaude\t\t7\n2\tcodex\t\t8\n",
        )
        .unwrap();
    });
    let id = sandbox.json(&["show", "#7", "--json"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let out = sandbox.home.path().join("prompt-out");
    let script = sandbox.home.path().join("legacy-agent.sh");
    fs::write(&script, format!("printf %s \"$2\" > '{}'\n", out.display())).unwrap();
    let launched = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .env("WS_AGENT_SHELL", "/bin/sh -c")
        .args(["_agent", "7", &format!("/bin/sh '{}'", script.display())])
        .output()
        .unwrap();
    assert!(launched.status.success(), "{launched:?}");
    assert_eq!(fs::read_to_string(&out).unwrap(), "hello legacy");
    assert!(!sandbox.home.path().join(".ws/sessions/7/prompt").exists());
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    let wt = sandbox.worktree();
    let ended = sandbox
        .command()
        .args(["_end", wt.to_str().unwrap(), "1", "7", &pane])
        .output()
        .unwrap();
    assert!(ended.status.success(), "{ended:?}");
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    assert_eq!(
        fs::read_to_string(sandbox.slots()).unwrap(),
        "2\tcodex\t\t8\n"
    );
}

#[test]
fn legacy_end_records_the_exit_of_a_zsh_launched_agent() {
    let mut pane = String::new();
    let sandbox = Sandbox::with_setup("", |home, server| {
        let repo = home.join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        let wt = home.join("worktrees/api/feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        let ids = Command::new("tmux")
            .args([
                "-S",
                server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "legacy",
            ])
            .args(["-P", "-F", "#{window_id}\t#{pane_id}", "/bin/cat"])
            .output()
            .unwrap();
        assert!(ids.status.success(), "{ids:?}");
        let ids = String::from_utf8(ids.stdout).unwrap();
        let (window, legacy_pane) = ids.trim_end().split_once('\t').unwrap();
        pane = legacy_pane.into();
        for (key, value) in [("@ws_session", "7"), ("@ws_pane", legacy_pane)] {
            let status = Command::new("tmux")
                .args(["-S", server, "set-option", "-w", "-t", window, key, value])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let legacy = home.join(".ws/sessions/7");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(home.join(".ws/sessions/counter"), "7").unwrap();
        fs::write(
            legacy.join("meta"),
            json!({"agent": "claude", "workspace": "api/feature", "window": window}).to_string(),
        )
        .unwrap();
        let git_dir = git(&wt, &["rev-parse", "--absolute-git-dir"]);
        fs::write(Path::new(&git_dir).join("ws-sessions"), "1\tclaude\t\t7\n").unwrap();
    });
    let id = sandbox.json(&["show", "#7", "--json"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let wt = sandbox.worktree();
    let ended = sandbox
        .command()
        .args(["_end", wt.to_str().unwrap(), "1", "7", &pane])
        .output()
        .unwrap();
    assert!(ended.status.success(), "{ended:?}");
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    assert_eq!(fs::read_to_string(sandbox.slots()).unwrap(), "");
}

#[test]
fn a_clean_exit_releases_its_slot_and_reopening_does_not_steal_another_slot() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (first, mut connection) = sandbox.capture();
    let id = first["id"].as_str().unwrap();
    connection.write_all(b"0").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if sandbox.json(&["show", id, "--json"])["state"] == "finished" {
            break;
        }
        assert!(
            Instant::now() < end,
            "The agent exit did not reach the daemon."
        );
    }
    assert!(
        sandbox
            .output(&["add", "api/feature", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    let (second, _running) = sandbox.capture();
    let second_id = second["id"].as_str().unwrap();
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(
        rows.starts_with(&format!("1\tcodex\t\t{second_id}\n")),
        "{rows}"
    );
    assert!(sandbox.output(&["open", id, "--detach"]).status.success());
    let (reopened, _running) = sandbox.capture();
    assert_eq!(reopened["id"], id);
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(rows.contains(&format!("1\tcodex\t\t{second_id}\n")));
    assert!(rows.contains(&format!("2\tclaude\t\t{id}\n")));
}

#[test]
fn a_manual_tab_can_run_an_agent_after_it_moves_to_another_tmux_session() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    fs::create_dir_all(sandbox.worktree().parent().unwrap()).unwrap();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "feature",
            sandbox.worktree().to_str().unwrap(),
        ],
    );
    assert!(sandbox.output(&["pin", "api/feature"]).status.success());
    let window = sandbox.tmux(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{window_id}",
        "-s",
        "custom",
        "-c",
        sandbox.worktree().to_str().unwrap(),
        "/bin/cat",
    ]);
    sandbox.tmux(&[
        "set-option",
        "-w",
        "-t",
        &window,
        "@ws_path",
        sandbox.worktree().to_str().unwrap(),
    ]);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    let child = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .current_dir(sandbox.worktree())
        .arg("claude")
        .spawn()
        .unwrap();
    let (capture, mut connection) = sandbox.capture();
    assert_eq!(sandbox.window(capture["id"].as_str().unwrap()), window);
    connection.write_all(b"0").unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
}

#[test]
fn split_panes_cannot_prepare_or_finish_the_windows_agent() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (capture, _running) = sandbox.capture();
    let id = capture["id"].as_str().unwrap();
    let window = sandbox.window(id);
    let pane = sandbox.tmux(&[
        "split-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        &window,
        "/bin/cat",
    ]);
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/launch",
        &json!({"id":id, "pane":pane}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 400"), "{result}");
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({"id":id, "pane":pane, "code":0, "user_tab":true}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 200"), "{result}");
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
    assert!(fs::read_to_string(sandbox.slots()).unwrap().contains(id));
}

#[test]
fn interactive_spawn_limits_preserve_slots_and_allow_existing_sessions_to_resume() {
    for (setting, message) in [
        ("max_children", "child limit"),
        ("max_depth", "depth limit"),
    ] {
        let sandbox = Sandbox::with_config(&format!("[limits]\n{setting} = 0\n"));
        let repo = sandbox.repo();
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    "feature",
                    "--detach"
                ])
                .status
                .success()
        );
        let (owner, _running) = sandbox.capture();
        let id = owner["id"].as_str().unwrap();
        let slots = fs::read_to_string(sandbox.slots()).unwrap();
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["add", "api/feature", "--detach"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{output:?}"
        );
        assert_eq!(fs::read_to_string(sandbox.slots()).unwrap(), slots);
        assert_eq!(
            sandbox
                .json(&["sessions", "--json"])
                .as_array()
                .unwrap()
                .len(),
            1
        );
        sandbox.tmux(&["kill-window", "-t", &sandbox.window(id)]);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["open", id, "--detach"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let (resumed, _running) = sandbox.capture();
        assert_eq!(resumed["id"], id);
    }
}

impl Sandbox {
    fn new() -> Self {
        Self::with_config("")
    }
    fn with_config(config: &str) -> Self {
        Self::with_setup(config, |_, _| {})
    }
    /// Runs `setup(home, tmux server)` before the daemon starts and imports legacy state.
    fn with_setup(config: &str, setup: impl FnOnce(&Path, &str)) -> Self {
        let home = common::SandboxRoot::new();
        fs::create_dir(home.path().join("state")).unwrap();
        fs::write(home.path().join("state/config.toml"), config).unwrap();
        let socket = home.path().join("wsd.sock");
        let server = home.tmux_socket();
        setup(home.path(), &server);
        let mut command = common::ws(home.path(), &socket);
        let listener = UnixListener::bind(home.path().join("agent.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        fs::write(
            home.path().join("agent shell.sh"),
            "export WS_TEST_AGENT_SHELL=ready\nexec /bin/sh -c \"$1\"\n",
        )
        .unwrap();
        command.env(
            "WS_AGENT_SHELL",
            format!(
                "/bin/sh {}",
                shell_words::quote(&home.path().join("agent shell.sh").to_string_lossy())
            ),
        );
        let bin = home.path().join("fake-agents");
        fs::create_dir_all(&bin).unwrap();
        for agent in ["claude", "codex", "grok"] {
            let script = format!(
                r#"#!/usr/bin/python3
import json, os, socket, subprocess, sys, tty
s = socket.socket(socket.AF_UNIX)
s.connect({:?})
s.sendall((json.dumps(dict(agent={agent:?}, argv=sys.argv[1:], id=os.getenv('WS_SESSION_ID'), legacy_id=os.getenv('WS_SESSION'), cwd=os.getcwd(), workspace=os.getenv('WS_WORKSPACE'), shell=os.getenv('WS_TEST_AGENT_SHELL'), pane=os.getenv('TMUX_PANE')))+'\n').encode())
control = s.recv(1)
if control == b'd':
    tty.setraw(0)
    os.write(1, b'\x1b[?2004h')
    data = b''
    while not data.endswith(b'\r'):
        data += os.read(0, 4096)
    text = data.removeprefix(b'\x1b[200~').removesuffix(b'\r').removesuffix(b'\x1b[201~').decode()
    s.sendall((text.encode('unicode_escape').decode()+'\n').encode())
    if s.recv(1) == b'a':
        subprocess.run([os.environ['WS_BIN'], 'hook', 'working'], input=json.dumps(dict(hook_event_name='UserPromptSubmit', turn_id='next-turn', session_id='resume-{agent}')).encode(), check=True)
    s.recv(1)
    sys.exit(0)
sys.exit(int(control or b'0'))
"#,
                home.path().join("agent.sock").to_str().unwrap()
            );
            fs::write(bin.join(agent), script).unwrap();
            fs::set_permissions(bin.join(agent), fs::Permissions::from_mode(0o755)).unwrap();
        }
        command
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("WS_ROOT", home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        let daemon = common::Daemon::start(command, &socket);
        assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
        let sandbox = Self {
            home,
            socket,
            server,
            listener,
            _daemon: daemon,
        };
        sandbox.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keepalive",
            "/bin/cat",
        ]);
        sandbox
    }
    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env(
                "WS_AGENT_SHELL",
                format!(
                    "/bin/sh {}",
                    shell_words::quote(&self.home.path().join("agent shell.sh").to_string_lossy())
                ),
            )
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        command
    }
    fn output(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let out = self.output(args);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-S", &self.server])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim_end().into()
    }
    fn capture(&self) -> (Value, UnixStream) {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(30)))
                        .unwrap();
                    let mut bytes = vec![];
                    loop {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        if byte[0] == b'\n' {
                            break;
                        }
                        bytes.push(byte[0]);
                    }
                    return (serde_json::from_slice(&bytes).unwrap(), stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "The fake agent did not start.");
                    std::thread::yield_now();
                }
                Err(e) => panic!("{e}"),
            }
        }
    }
    fn wait_for(&self, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "The expected event did not occur."
            );
            std::thread::yield_now();
        }
    }
    fn log(&self, id: &str) -> Vec<ws_core::log::LogEntry> {
        ws_core::log::SessionLog::new(
            &self.home.path().join("state"),
            id.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries
    }
    fn events(&self, id: &str, event: &str) -> Vec<ws_core::log::LogEntry> {
        self.log(id)
            .into_iter()
            .filter(|entry| entry.event_type == event)
            .collect()
    }
    fn repo(&self) -> PathBuf {
        let repo = self.home.path().join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        repo
    }
    fn worktree(&self) -> PathBuf {
        self.home.path().join("worktrees/api/feature")
    }
    fn slots(&self) -> PathBuf {
        PathBuf::from(git(&self.worktree(), &["rev-parse", "--absolute-git-dir"]))
            .join("ws-sessions")
    }
    fn window(&self, id: &str) -> String {
        self.tmux(&["list-windows", "-a", "-F", "#{window_id}\t#{@ws_session}"])
            .lines()
            .find_map(|l| {
                let (w, s) = l.split_once('\t')?;
                (s == id).then(|| w.into())
            })
            .unwrap()
    }
}
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap().trim_end().into()
}

fn assert_mcp_registration(args: &[String], agent: &str, id: &str) {
    if agent == "claude" {
        let index = args.iter().position(|arg| arg == "--mcp-config").unwrap();
        let config: Value = serde_json::from_slice(&fs::read(&args[index + 1]).unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["ws"]["command"],
            env!("CARGO_BIN_EXE_ws")
        );
        assert_eq!(config["mcpServers"]["ws"]["args"], json!(["mcp"]));
        assert_eq!(config["mcpServers"]["ws"]["env"]["WS_SESSION_ID"], id);
    } else if agent == "codex" {
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", "mcp_servers.ws.args=[\"mcp\"]"])
        );
        assert!(
            args.iter()
                .any(|arg| arg == &format!("mcp_servers.ws.env.WS_SESSION_ID=\"{id}\""))
        );
    }
}
