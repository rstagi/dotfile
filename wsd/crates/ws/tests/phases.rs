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
fn phase_spawn_runs_a_local_child_and_records_its_parent_and_phase_context() {
    let sandbox = Sandbox::new();
    let parent = sandbox.request("POST", "/v1/sessions", json!({}));
    let request = json!({
        "parent_session_id": parent["id"],
        "phase": {"command": ["/bin/sh", "-c", "pwd; printf '%s' \"$WS_SESSION_ID\""]},
        "repo": sandbox.home.path(), "base": "main",
        "caps_required": [], "max_runtime": 60
    });
    let child = sandbox.request("POST", "/v1/phases/spawn", request.clone());
    let id = child["id"].as_str().unwrap();
    let child = sandbox.wait(id);
    assert_eq!(child["state"], "finished");
    assert_eq!(child["exit_code"], 0);
    assert_eq!(child["body"]["parent_session_id"], parent["id"]);
    assert_eq!(child["body"]["phase"], request["phase"]);
    assert_eq!(child["body"]["base"], "main");
    assert_eq!(child["body"]["caps_required"], json!([]));
    let stdout = fs::read_to_string(
        sandbox
            .home
            .path()
            .join("state/sessions")
            .join(id)
            .join("stdout.log"),
    )
    .unwrap();
    assert_eq!(
        stdout,
        format!(
            "{}\n{id}",
            sandbox.home.path().canonicalize().unwrap().display()
        )
    );
    let entries = sandbox.entries(id);
    let running = entries
        .iter()
        .find(|entry| entry.event_type == "session.running")
        .unwrap();
    assert!(running.body["process"]["pid"].as_u64().unwrap() > 1);
    assert_eq!(
        running.body["process"]["pid"],
        running.body["process"]["pgid"]
    );
    let parent_entries = sandbox.entries(parent["id"].as_str().unwrap());
    assert!(
        parent_entries
            .iter()
            .any(|entry| entry.event_type == "phase.spawned" && entry.body["session_id"] == id)
    );
}

#[test]
fn restart_adopts_a_running_child_and_fails_a_child_that_disappeared() {
    let mut sandbox = Sandbox::new();
    let running = sandbox.headless("live");
    let dead = sandbox.headless("dead");
    sandbox.wait_output(&running, "ready");
    sandbox.wait_output(&dead, "ready");
    let dead_pid = sandbox.process(&dead)["pid"].as_u64().unwrap();
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    kill_group(dead_pid);
    sandbox.wait_gone(dead_pid);
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    let failed = sandbox.wait(&dead);
    assert_eq!(failed["state"], "failed");
    assert!(failed["exit_code"].is_null());
    assert!(sandbox.entries(&dead).iter().any(|entry| {
        entry.event_type == "session.failed"
            && entry.body["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
    }));
    assert_eq!(sandbox.get(&running)["state"], "running");
    assert!(
        sandbox
            .entries(&running)
            .iter()
            .any(|entry| entry.event_type == "process.adopted")
    );
    fs::write(sandbox.home.path().join("release-live"), "release").unwrap();
    // The new daemon cannot obtain an exit status from waitpid for an adopted process.
    assert_eq!(sandbox.wait(&running)["state"], "failed");
    assert!(
        sandbox
            .entries(&running)
            .iter()
            .any(|entry| entry.event_type == "exit"
                && entry.body["code"].is_null()
                && entry.body["reason"].is_string())
    );
}

fn kill_group(pid: u64) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &format!("-{pid}")])
        .output();
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.daemon.child.kill();
        let _ = self.daemon.child.wait();
        if let Ok(directories) = fs::read_dir(self.home.path().join("state/sessions")) {
            for directory in directories.flatten() {
                let id = directory.file_name().to_string_lossy().into_owned();
                if let Ok(id) = id.parse() {
                    let log = SessionLog::new(&self.home.path().join("state"), id, Ulid::new());
                    if let Ok(log) = log.read() {
                        for entry in log.entries {
                            if entry.event_type == "session.running"
                                && let Some(pid) = entry.body["process"]["pid"].as_u64()
                                && pid > 1
                            {
                                kill_group(pid);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
        Self {
            daemon,
            home,
            socket,
        }
    }

    fn headless(&self, name: &str) -> String {
        let script = format!("printf ready; while [ ! -e release-{name} ]; do sleep 0.02; done");
        let session = self.request(
            "POST",
            "/v1/sessions",
            json!({
                "headless": true, "command": ["/bin/sh", "-c", script], "cwd": self.home.path(),
            }),
        );
        session["id"].as_str().unwrap().to_owned()
    }

    fn process(&self, id: &str) -> Value {
        self.entries(id)
            .into_iter()
            .find(|entry| entry.event_type == "session.running")
            .unwrap()
            .body["process"]
            .clone()
    }

    fn wait_output(&self, id: &str, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let path = self
            .home
            .path()
            .join("state/sessions")
            .join(id)
            .join("stdout.log");
        while !fs::read_to_string(&path).is_ok_and(|text| text.contains(expected)) {
            assert!(
                Instant::now() < deadline,
                "The child did not write {expected}."
            );
            thread::yield_now();
        }
    }

    fn wait_gone(&self, pid: u64) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            if !output.status.success() || output.stdout.starts_with(b"Z") {
                return;
            }
            assert!(Instant::now() < deadline, "The child is still running.");
            thread::yield_now();
        }
    }

    fn request(&self, method: &str, path: &str, body: Value) -> Value {
        let response = common::request(&self.socket, method, path, &body.to_string());
        assert!(
            response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.1 201"),
            "{response}"
        );
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn get(&self, id: &str) -> Value {
        self.request("GET", &format!("/v1/sessions/{id}"), Value::Null)
    }

    fn wait(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let session = self.get(id);
            if ["finished", "failed"].contains(&session["state"].as_str().unwrap()) {
                return session;
            }
            assert!(
                Instant::now() < deadline,
                "The child did not exit: {session}"
            );
            thread::yield_now();
        }
    }

    fn entries(&self, id: &str) -> Vec<ws_core::log::LogEntry> {
        SessionLog::new(
            &self.home.path().join("state"),
            id.parse().unwrap(),
            Ulid::new(),
        )
        .read()
        .unwrap()
        .entries
    }
}
