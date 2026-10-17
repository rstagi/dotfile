use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use ws_core::{ids::Ulid, log::SessionLog};

#[allow(dead_code)]
mod common;

struct Sandbox {
    _daemon: common::Daemon,
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

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
        Self {
            _daemon: daemon,
            home,
            socket,
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
