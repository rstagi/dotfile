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
fn phase_worktree_is_detached_hidden_from_list_and_removed_on_stop() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let session = sandbox.request("POST", "/v1/sessions", json!({}));
    let id = session["id"].as_str().unwrap();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .current_dir(sandbox.home.path())
        .args(["phase", "worktree", "--session", id, "repo", "HEAD"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert_eq!(
        path,
        sandbox
            .home
            .path()
            .canonicalize()
            .unwrap()
            .join("phases")
            .join(id)
    );
    assert_eq!(
        git(&path, &["rev-parse", "HEAD"]),
        git(&repo, &["rev-parse", "HEAD"])
    );
    assert!(
        !Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["symbolic-ref", "-q", "HEAD"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        sandbox
            .entries(id)
            .iter()
            .any(|entry| entry.event_type == "phase.worktree"
                && entry.body["path"] == path.to_str().unwrap()
                && entry.body["repo"] == repo.canonicalize().unwrap().to_str().unwrap())
    );
    let sessions = common::ws(sandbox.home.path(), &sandbox.socket)
        .args(["sessions", id, "--json"])
        .output()
        .unwrap();
    assert!(sessions.status.success(), "{sessions:?}");
    let sessions: Value = serde_json::from_slice(&sessions.stdout).unwrap();
    assert_eq!(
        sessions[0]["repos"],
        json!([
            {"path": path, "base": "HEAD"}
        ])
    );
    let list = common::ws(sandbox.home.path(), &sandbox.socket)
        .env("WS_SESSION_ID", id)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&list.stdout).unwrap(),
        json!([])
    );
    fs::write(path.join("untracked"), "review output").unwrap();
    sandbox.request("POST", &format!("/v1/sessions/{id}/stop"), json!({}));
    assert!(!path.exists());
    assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn invalid_phase_repository_does_not_prevent_daemon_restart() {
    let mut sandbox = Sandbox::new();
    let session = sandbox.request("POST", "/v1/sessions", json!({}));
    let id = session["id"].as_str().unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/phases/worktree",
        &json!({
            "session": id, "repo": sandbox.home.path(), "reference": "HEAD",
        })
        .to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 500"), "{response}");
    sandbox.request("POST", &format!("/v1/sessions/{id}/stop"), json!({}));
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    assert_eq!(sandbox.get(id)["state"], "finished");
}

#[test]
fn phase_worktree_uses_current_session_and_cleans_up_after_process_exit() {
    for code in [0, 3] {
        let sandbox = Sandbox::new();
        let repo = sandbox.repo();
        let session = sandbox.request("POST", "/v1/sessions", json!({
            "headless": true,
            "command": ["/bin/sh", "-c", format!("printf ready; while [ ! -e release ]; do sleep 0.02; done; exit {code}")],
            "cwd": sandbox.home.path(),
        }));
        let id = session["id"].as_str().unwrap();
        sandbox.wait_output(id, "ready");
        let output = common::ws(sandbox.home.path(), &sandbox.socket)
            .env("WS_SESSION_ID", id)
            .args(["phase", "worktree", repo.to_str().unwrap(), "HEAD"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        fs::write(path.join("untracked"), "review output").unwrap();
        fs::write(sandbox.home.path().join("release"), "release").unwrap();
        let session = sandbox.wait(id);
        assert_eq!(
            session["state"],
            if code == 0 { "finished" } else { "failed" }
        );
        assert!(!path.exists());
        assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
    }
}

#[test]
fn restart_collects_a_phase_worktree_after_an_interrupted_terminal_event() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let session = sandbox.request("POST", "/v1/sessions", json!({}));
    let id = session["id"].as_str().unwrap();
    let path = sandbox.request(
        "POST",
        "/v1/phases/worktree",
        json!({
            "session": id, "repo": repo, "reference": "HEAD",
        }),
    );
    let path = PathBuf::from(path.as_str().unwrap());
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    // Simulate a crash after the terminal event reaches the log, before cleanup.
    SessionLog::new(
        &sandbox.home.path().join("state"),
        id.parse().unwrap(),
        session["device_id"].as_str().unwrap().parse().unwrap(),
    )
    .writer()
    .append_current("session.finished", json!({}))
    .unwrap();
    assert!(path.exists());
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    assert_eq!(sandbox.get(id)["state"], "finished");
    assert!(!path.exists());
    assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
}

#[test]
fn cleanup_failure_does_not_block_startup_and_a_later_restart_retries_it() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let session = sandbox.request("POST", "/v1/sessions", json!({}));
    let id = session["id"].as_str().unwrap();
    let path = sandbox.request(
        "POST",
        "/v1/phases/worktree",
        json!({
            "session": id, "repo": repo, "reference": "HEAD",
        }),
    );
    let path = PathBuf::from(path.as_str().unwrap());
    let moved = sandbox.home.path().join("repo-unavailable");
    fs::rename(&repo, &moved).unwrap();
    sandbox.request("POST", &format!("/v1/sessions/{id}/stop"), json!({}));
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    assert_eq!(sandbox.get(id)["state"], "finished");
    assert!(path.exists());
    assert!(
        sandbox
            .daemon
            .stderr()
            .contains("Cannot remove the phase worktree")
    );
    fs::rename(&moved, &repo).unwrap();
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    assert!(!path.exists());
    assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
}

#[test]
fn a_reused_phase_path_does_not_leave_a_registration_in_the_previous_repo() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let other = sandbox.home.path().join("other-repo");
    git(
        sandbox.home.path(),
        &["clone", repo.to_str().unwrap(), other.to_str().unwrap()],
    );
    let session = sandbox.request("POST", "/v1/sessions", json!({}));
    let id = session["id"].as_str().unwrap();
    let path = sandbox.request(
        "POST",
        "/v1/phases/worktree",
        json!({
            "session": id, "repo": repo, "reference": "HEAD",
        }),
    );
    let path = PathBuf::from(path.as_str().unwrap());
    // Represent a lost directory after an interrupted checkout or external removal.
    fs::remove_dir_all(&path).unwrap();
    let reused = sandbox.request(
        "POST",
        "/v1/phases/worktree",
        json!({
            "session": id, "repo": other, "reference": "HEAD",
        }),
    );
    assert_eq!(reused, path.to_str().unwrap());
    assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
    sandbox.request("POST", &format!("/v1/sessions/{id}/stop"), json!({}));
    assert!(!path.exists());
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
    assert_eq!(child["parent_session_id"], parent["id"]);
    let id = child["id"].as_str().unwrap();
    let child = sandbox.wait(id);
    assert_eq!(child["state"], "finished");
    assert_eq!(child["exit_code"], 0);
    assert_eq!(child["body"]["parent_session_id"], parent["id"]);
    assert_eq!(child["body"]["phase"], request["phase"]);
    assert_eq!(child["body"]["base"], "main");
    assert_eq!(child["body"]["caps_required"], json!([]));
    let listed = common::ws(sandbox.home.path(), &sandbox.socket)
        .args(["sessions", id, "--json"])
        .output()
        .unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(
        listed[0]["repos"],
        json!([
            {"path": sandbox.home.path().canonicalize().unwrap(), "base": "main"}
        ])
    );
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
            .any(|entry| entry.event_type == "api.called"
                && entry.body["operation"] == "phase.spawn")
    );
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

#[test]
fn restart_enforces_a_persisted_phase_deadline_that_already_expired() {
    let mut sandbox = Sandbox::new();
    let parent = sandbox.request("POST", "/v1/sessions", json!({}));
    let child = sandbox.request("POST", "/v1/phases/spawn", json!({
        "parent_session_id": parent["id"],
        "phase": {"command": ["/bin/sh", "-c", "trap 'printf term' TERM; printf ready; while :; do sleep 1; done"]},
        "repo": sandbox.home.path(), "base": "main", "caps_required": [], "max_runtime": 3600,
    }));
    let id = child["id"].as_str().unwrap();
    sandbox.wait_output(id, "ready");
    let mut process = sandbox.process(id);
    assert!(process["deadline_ms"].as_u64().unwrap() > 1);
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    // Represent a restart after the original deadline. The command still has a long runtime in its creation metadata.
    process["deadline_ms"] = json!(1);
    let log = SessionLog::new(
        &sandbox.home.path().join("state"),
        id.parse().unwrap(),
        child["device_id"].as_str().unwrap().parse().unwrap(),
    );
    log.writer()
        .append_current(
            "session.running",
            json!({"process": process, "parent_session_id": parent["id"]}),
        )
        .unwrap();
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    assert_eq!(sandbox.wait(id)["state"], "failed");
    sandbox.wait_output(id, "term");
    sandbox.wait_gone(process["pid"].as_u64().unwrap());
    assert!(
        sandbox
            .entries(id)
            .iter()
            .any(|entry| entry.event_type == "session.failed"
                && entry.body["reason"] == "The phase exceeded its maximum runtime.")
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

    fn repo(&self) -> PathBuf {
        let repo = self.home.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        repo
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
