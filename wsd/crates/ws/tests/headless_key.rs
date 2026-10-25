use std::{
    fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use ws_core::{ids::Ulid, log::SessionLog};

#[allow(dead_code)]
mod common;

struct Sandbox {
    daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
}

#[test]
fn a_retry_after_a_lost_response_returns_the_first_session_and_runs_once() {
    let sandbox = Sandbox::new();
    let parent = sandbox.create(&[], "true");
    let first = sandbox.create(&["--parent", &parent, "--key", "k1"], &sandbox.script(1, ""));
    sandbox.wait_state(&first, "finished");
    // The client lost the response, so it sends the same request again.
    let retry = sandbox.create(&["--parent", &parent, "--key", "k1"], &sandbox.script(1, ""));
    assert_eq!(retry, first);
    // A terminal result stays the result of the key. A new execution needs a new key.
    assert_eq!(sandbox.runs(1), 1);
    let other = sandbox.create(&["--parent", &parent, "--key", "k2"], &sandbox.script(2, ""));
    assert_ne!(other, first);
    sandbox.wait_state(&other, "finished");
    assert_eq!(sandbox.runs(2), 1);
    assert_eq!(sandbox.entries(&first, "session.created"), 1);
    assert_eq!(sandbox.get(&first)["body"]["key"], "k1");
}

#[test]
fn the_same_key_under_another_parent_starts_another_session() {
    let sandbox = Sandbox::new();
    let one = sandbox.create(&[], "true");
    let two = sandbox.create(&[], "true");
    let a = sandbox.create(&["--parent", &one, "--key", "k"], "true");
    let b = sandbox.create(&["--parent", &two, "--key", "k"], "true");
    let c = sandbox.create(&["--parent", &one, "--key", "k"], "true");
    assert_ne!(a, b);
    assert_eq!(a, c);
}

#[test]
fn a_key_survives_a_crash_after_the_launch_and_the_process_runs_once() {
    let mut sandbox = Sandbox::new();
    let parent = sandbox.create(&[], "true");
    let script = sandbox.script(1, "sleep 120");
    let first = sandbox.create(&["--parent", &parent, "--key", "k"], &script);
    sandbox.wait_runs(1, 1);
    sandbox.restart();
    assert_eq!(sandbox.get(&first)["state"], "running");
    let retry = sandbox.create(&["--parent", &parent, "--key", "k"], &script);
    assert_eq!(retry, first);
    assert_eq!(sandbox.runs(1), 1);
    let stop = sandbox.command().args(["session", "stop", &first]).output().unwrap();
    assert!(stop.status.success(), "{stop:?}");
    assert_eq!(sandbox.create(&["--parent", &parent, "--key", "k"], &script), first);
    assert_eq!(sandbox.runs(1), 1);
}

#[test]
fn a_reservation_without_a_process_survives_a_crash_and_starts_once() {
    let mut sandbox = Sandbox::new();
    let parent = sandbox.create(&[], "true");
    // A daemon that crashed after the reservation left only this record.
    let reference = sandbox.entry(&parent, "session.created");
    let id = Ulid::new();
    let state = sandbox.home.path().join("state");
    fs::create_dir_all(state.join("sessions").join(id.to_string())).unwrap();
    sandbox.stop_daemon();
    SessionLog::new(&state, id, reference.device_id)
        .append(
            reference.epoch,
            "session.created",
            json!({
                "alias": 99, "kind": "headless", "command": ["true"],
                "cwd": sandbox.home.path(), "key": "k",
                "stdout": state.join("sessions").join(id.to_string()).join("stdout.log"),
                "stderr": state.join("sessions").join(id.to_string()).join("stderr.log"),
                "parent_session_id": parent,
            }),
        )
        .unwrap();
    sandbox.start_daemon();
    assert_eq!(sandbox.get(&id.to_string())["state"], "created");
    let script = sandbox.script(1, "");
    let retry = sandbox.create(&["--parent", &parent, "--key", "k"], &script);
    assert_eq!(retry, id.to_string());
    sandbox.wait_state(&retry, "finished");
    assert_eq!(sandbox.create(&["--parent", &parent, "--key", "k"], &script), retry);
    assert_eq!(sandbox.runs(1), 1);
    assert_eq!(sandbox.entries(&retry, "session.created"), 1);
    assert_eq!(sandbox.entries(&retry, "session.running"), 1);
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        fs::create_dir(home.path().join("state")).unwrap();
        let socket = home.path().join("wsd.sock");
        let mut command = common::ws(home.path(), &socket);
        command.current_dir(home.path()).env("WS_STATE", "state");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            daemon,
            home,
            socket,
        }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .current_dir(self.home.path())
            .env("WS_STATE", "state")
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command
    }

    fn stop_daemon(&mut self) {
        self.daemon.child.kill().unwrap();
        self.daemon.child.wait().unwrap();
    }

    fn start_daemon(&mut self) {
        self.daemon = common::Daemon::start(self.command(), &self.socket);
    }

    fn restart(&mut self) {
        self.stop_daemon();
        self.start_daemon();
    }

    /// A shell script that counts its runs in `run<n>`, then runs the tail.
    fn script(&self, n: u32, tail: &str) -> String {
        format!(
            "echo x >> '{}'; {tail}",
            self.home.path().join(format!("run{n}")).display()
        )
    }

    fn runs(&self, n: u32) -> usize {
        fs::read_to_string(self.home.path().join(format!("run{n}")))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    fn wait_runs(&self, n: u32, count: usize) {
        self.wait_until(|| self.runs(n) >= count);
    }

    fn create(&self, flags: &[&str], script: &str) -> String {
        let output = self
            .command()
            .args(["session", "create", "--headless"])
            .args(flags)
            .args(["--", "/bin/sh", "-c", script])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn get(&self, id: &str) -> Value {
        let response = common::get(&self.socket, &format!("/v1/sessions/{id}"));
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn wait_state(&self, id: &str, state: &str) {
        self.wait_until(|| self.get(id)["state"] == state);
    }

    fn wait_until(&self, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !check() {
            assert!(Instant::now() < deadline, "The condition did not occur.");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn log(&self, id: &str) -> Vec<ws_core::log::LogEntry> {
        SessionLog::new(
            &self.home.path().join("state"),
            id.parse().unwrap(),
            Ulid::new(),
        )
        .read()
        .unwrap()
        .entries
    }

    fn entries(&self, id: &str, event_type: &str) -> usize {
        self.log(id)
            .iter()
            .filter(|entry| entry.event_type == event_type)
            .count()
    }

    fn entry(&self, id: &str, event_type: &str) -> ws_core::log::LogEntry {
        self.log(id)
            .into_iter()
            .find(|entry| entry.event_type == event_type)
            .unwrap()
    }
}
