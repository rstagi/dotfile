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
    home: tempfile::TempDir,
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
    let output = sandbox
        .command()
        .args(["whoami"])
        .env("TMUX_PANE", &sandbox.window)
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

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let server = format!("ws-cli-{}-{}", std::process::id(), Ulid::new());
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
                "-L",
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

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.server, "kill-server"])
            .output();
    }
}
