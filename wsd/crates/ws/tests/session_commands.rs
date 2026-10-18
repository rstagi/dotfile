use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use serde_json::{Value, json};
use ws_core::{ids::Ulid, log::SessionLog};
use ws_daemon::tmux::Tmux;

#[allow(dead_code)]
mod common;

struct Sandbox {
    home: common::SandboxRoot,
    socket: PathBuf,
    server: String,
    ids: Vec<String>,
    window: String,
    _daemon: common::Daemon,
}

#[test]
fn sessions_matches_the_zsh_golden_output_and_resolves_names() {
    let sandbox = Sandbox::new();
    sandbox.assert_golden(&["sessions"], include_str!("golden/sessions.txt"));
    for target in ["finisher", "#2", &sandbox.ids[1], &sandbox.ids[1][..25]] {
        sandbox.assert_golden(&["sessions", target], include_str!("golden/selected.txt"));
    }
    let listed = sandbox.json(&["sessions", "--json"]);
    assert_eq!(listed.as_array().unwrap().len(), 3);
    assert_eq!(listed[0]["id"], sandbox.ids[0]);
    assert_eq!(listed[0]["alias"], 1);
    assert_eq!(listed[1]["state"], "finished");
    sandbox.assert_error(&["sessions", "absent"], include_str!("golden/missing.txt"));
    let tmux = Tmux::named(&sandbox.server);
    tmux.set_state(&sandbox.window, "working").unwrap();
    let working = sandbox.run(&["sessions", "#1"]);
    assert_eq!(
        String::from_utf8_lossy(&working.stdout),
        "#1\t\tapi/seed\tclaude\tworking\tactive\n"
    );
    tmux.set_window_option(&sandbox.window, "@ws_session", "unrelated")
        .unwrap();
    let stopped = sandbox.run(&["sessions", "#1"]);
    assert_eq!(
        String::from_utf8_lossy(&stopped.stdout),
        "#1\t\tapi/seed\tclaude\tstopped\tactive\n"
    );
}

#[test]
fn sessions_keep_legacy_window_activity() {
    let sandbox = Sandbox::new();
    let tmux = Tmux::named(&sandbox.server);
    for identity in ["1", "#1", &sandbox.ids[0]] {
        tmux.set_window_option(&sandbox.window, "@ws_session", identity)
            .unwrap();
        for state in ["working", "idle"] {
            tmux.set_state(&sandbox.window, state).unwrap();
            sandbox.assert_golden(
                &["sessions", "#1"],
                &format!("#1\t\tapi/seed\tclaude\t{state}\tactive\n"),
            );
        }
    }
}

#[test]
fn legacy_agents_create_children_with_their_resolved_parent() {
    let sandbox = Sandbox::new();
    for legacy in ["1", "#1", &sandbox.ids[0]] {
        for headless in [false, true] {
            let mut command = sandbox.command();
            command
                .env("WS_SESSION", legacy)
                .args(["session", "create", "--json"]);
            if headless {
                command.args(["--headless", "--", "/usr/bin/true"]);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let child: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(child["parent_session_id"], sandbox.ids[0]);
            if headless {
                let exited = sandbox.run(&[
                    "wait",
                    child["id"].as_str().unwrap(),
                    "--for",
                    "event=process.exited",
                    "--timeout",
                    "30",
                ]);
                assert!(exited.status.success(), "{exited:?}");
            }
        }
    }
}

#[test]
fn readme_session_create_examples_use_supported_name_and_headless_options() {
    let sandbox = Sandbox::new();
    let readme = include_str!("../../../../ws/README.md");
    let row = readme
        .lines()
        .find(|line| line.starts_with("| `session_create` |"))
        .unwrap();
    for example in row
        .split('`')
        .filter(|example| example.starts_with("ws session create"))
    {
        // Exercise every optional flag in each documented command form.
        let expanded = example
            .replace(['[', ']'], "")
            .replace("<name>", "doc-example")
            .replace('…', "")
            .replace("<cmd>", "/usr/bin/true");
        let args: Vec<_> = expanded.split_whitespace().skip(1).collect();
        let output = sandbox.run(&args);
        assert!(output.status.success(), "{example}: {output:?}");
        let child: Value = serde_json::from_slice(&output.stdout).unwrap();
        if args.contains(&"--headless") {
            assert!(child["name"].is_null());
            let exited = sandbox.run(&[
                "wait",
                child["id"].as_str().unwrap(),
                "--for",
                "event=process.exited",
                "--timeout",
                "30",
            ]);
            assert!(exited.status.success(), "{exited:?}");
        } else {
            assert_eq!(child["name"], "doc-example");
        }
    }
    for option in ["--name", "--workspace-id"] {
        let rejected = sandbox.run(&[
            "session",
            "create",
            "--headless",
            option,
            "unused",
            "--",
            "/usr/bin/true",
        ]);
        assert_eq!(rejected.status.code(), Some(1), "{rejected:?}");
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("does not accept name or workspace_id")
        );
    }
}

#[test]
fn whoami_uses_environment_precedence_then_the_tmux_window() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["whoami"])
        .env("WS_SESSION_ID", &sandbox.ids[0])
        .env("WS_SESSION", "#2")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("golden/whoami.txt")
    );
    let output = sandbox
        .command()
        .args(["whoami", "--json"])
        .env("WS_SESSION_ID", "")
        .env("WS_SESSION", "#2")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let identity: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(identity["id"], sandbox.ids[1]);
    assert_eq!(identity["alias"], 2);
    let pane = Command::new("tmux")
        .args([
            "-S",
            &sandbox.server,
            "display-message",
            "-p",
            "-t",
            &sandbox.window,
            "#{pane_id}",
        ])
        .output()
        .unwrap();
    assert!(pane.status.success());
    let pane = String::from_utf8(pane.stdout).unwrap();
    let output = sandbox
        .command()
        .args(["whoami"])
        .env("TMUX_PANE", pane.trim())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("golden/whoami.txt")
    );
    let output = sandbox
        .command()
        .args(["whoami"])
        .env("TMUX", "sandbox")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("golden/whoami.txt")
    );
    sandbox.assert_error(&["whoami"], include_str!("golden/whoami-outside.txt"));
    let invalid = sandbox
        .command()
        .args(["whoami"])
        .env("WS_SESSION_ID", "missing")
        .env("TMUX_PANE", &sandbox.window)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        include_str!("golden/whoami-outside.txt")
    );
    let me = sandbox
        .command()
        .args(["sessions", "me"])
        .env("WS_SESSION_ID", &sandbox.ids[1])
        .output()
        .unwrap();
    assert!(me.status.success(), "{me:?}");
    assert_eq!(
        String::from_utf8_lossy(&me.stdout),
        include_str!("golden/selected.txt")
    );
}

#[test]
fn show_matches_zsh_and_reports_the_durable_output_path() {
    let sandbox = Sandbox::new();
    let output_path = sandbox
        .home
        .path()
        .join("state/sessions")
        .join(&sandbox.ids[1])
        .join("output.md");
    let expected =
        include_str!("golden/show.txt").replace("<OUTPUT>", output_path.to_str().unwrap());
    sandbox.assert_golden(&["show", "finisher"], &expected);
    sandbox.assert_golden(&["show", "#1"], include_str!("golden/show-empty.txt"));
    let shown = sandbox.json(&["show", "--json", "#2"]);
    assert_eq!(shown["id"], sandbox.ids[1]);
    assert_eq!(shown["alias"], 2);
    assert_eq!(shown["name"], "finisher");
    assert_eq!(shown["output"], output_path.to_str().unwrap());
    sandbox.assert_error(&["show", "absent"], include_str!("golden/missing.txt"));
    sandbox.assert_error(&["show"], "ws: show: expected <id|name>\n");
}

#[test]
fn name_is_unique_validated_and_durable_across_daemon_restart() {
    let mut sandbox = Sandbox::new();
    let rename = |sandbox: &Sandbox, name: &str, json: bool| {
        let mut command = sandbox.command();
        command
            .args(["name", name])
            .env("WS_SESSION_ID", &sandbox.ids[0]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    let output = rename(&sandbox, "builder", false);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("golden/name.txt")
    );
    assert!(output.stderr.is_empty());
    let output = rename(&sandbox, "builder", true);
    assert!(output.status.success(), "{output:?}");
    let named: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(named["id"], sandbox.ids[0]);
    assert_eq!(named["name"], "builder");
    for invalid in ["me", "", "123", "bad name", "a/b", "éclair"] {
        let output = rename(&sandbox, invalid, false);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            include_str!("golden/name-invalid.txt")
        );
    }
    let output = rename(&sandbox, "finisher", false);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        include_str!("golden/name-taken.txt")
    );
    sandbox.assert_error(&["name"], "ws: name: expected <label>\n");
    sandbox.assert_error(
        &["name", "builder"],
        include_str!("golden/whoami-outside.txt"),
    );
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    let mut command = sandbox.command();
    command.env("WS_TMUX_SOCKET", &sandbox.server);
    sandbox._daemon = common::Daemon::start(command, &sandbox.socket);
    assert_eq!(
        sandbox.json(&["show", "builder", "--json"])["name"],
        "builder"
    );
    assert_eq!(
        sandbox.json(&["sessions", "builder", "--json"])[0]["id"],
        sandbox.ids[0]
    );
}

impl Sandbox {
    fn new() -> Self {
        let home = common::SandboxRoot::new();
        let socket = home.path().join("wsd.sock");
        let server = home.tmux_socket();
        let mut ids = Vec::new();
        let device = Ulid::new();
        for (alias, name, agent, state) in [
            (1, None, "claude", None),
            (2, Some("finisher"), "codex", Some("session.finished")),
            (3, Some("old"), "grok", Some("session.failed")),
        ] {
            let id = Ulid::new();
            let log = SessionLog::new(&home.path().join("state"), id, device);
            log.append(
                0,
                "session.created",
                json!({
                    "alias": alias, "name": name, "workspace_id": "api/seed", "agent": agent
                }),
            )
            .unwrap();
            if let Some(state) = state {
                log.append(0, state, json!({})).unwrap();
            }
            ids.push(id.to_string());
        }
        fs::write(
            home.path()
                .join("state/sessions")
                .join(&ids[1])
                .join("output.md"),
            "result\n",
        )
        .unwrap();
        let tmux = Tmux::named(&server);
        let created = Command::new("tmux")
            .args([
                "-S",
                &server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "fixture",
                "/bin/sleep",
                "300",
            ])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
        let window = tmux.window_for_pane("fixture:0.0").unwrap();
        tmux.set_window_option(&window, "@ws_session", &ids[0])
            .unwrap();
        tmux.set_state(&window, "idle").unwrap();
        let mut command = common::ws(home.path(), &socket);
        command.env("WS_TMUX_SOCKET", &server);
        let daemon = common::Daemon::start(command, &socket);
        Self {
            home,
            socket,
            server,
            ids,
            window,
            _daemon: daemon,
        }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env("WS_TMUX_SOCKET", &self.server)
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn assert_golden(&self, args: &[&str], expected: &str) {
        let output = self.run(args);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    fn assert_error(&self, args: &[&str], expected: &str) {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
