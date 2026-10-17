use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use ws_core::{ids::Ulid, log::SessionLog};

#[allow(dead_code)]
mod common;

struct Sandbox {
    daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
}

#[test]
fn stop_waits_for_a_graceful_exit_and_records_the_child_status() {
    let sandbox = Sandbox::new();
    let id = sandbox.create("trap 'exit 23' TERM; printf ready > ready; while :; do sleep 1; done");
    sandbox.wait_file("ready");
    let session = sandbox.stop(&id);
    assert_eq!(session["exit_code"], 23);
    assert_eq!(session["state"], "failed");
    assert!(
        sandbox
            .entries(&id)
            .iter()
            .any(|entry| entry.event_type == "exit")
    );
    sandbox.assert_group_gone(&id);
}

#[test]
fn stop_kills_a_child_that_ignores_sigterm() {
    let sandbox = Sandbox::new();
    let id = sandbox
        .create("trap 'printf term > term' TERM; printf ready > ready; while :; do sleep 1; done");
    sandbox.wait_file("ready");
    let session = sandbox.stop(&id);
    assert_eq!(session["exit_code"], 137);
    assert_eq!(sandbox.wait_file("term"), "term");
    sandbox.assert_group_gone(&id);
}

#[test]
fn stop_waits_for_a_grandchild_after_the_group_leader_exits() {
    let sandbox = Sandbox::new();
    let id = sandbox.create("trap 'exit 23' TERM; /bin/sh -c 'trap \"\" TERM; echo $$ > grandchild; while :; do sleep 1; done' & wait");
    let grandchild = sandbox.wait_file("grandchild");
    let session = sandbox.stop(&id);
    assert_eq!(session["exit_code"], 23);
    sandbox.assert_group_gone(&id);
    let alive = Command::new("/bin/kill")
        .args(["-0", grandchild.trim()])
        .output()
        .unwrap();
    assert!(!alive.status.success());
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

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .current_dir(self.home.path())
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION");
        command
    }

    fn create(&self, script: &str) -> String {
        let output = self
            .command()
            .args([
                "session",
                "create",
                "--headless",
                "--",
                "/bin/sh",
                "-c",
                script,
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn stop(&self, id: &str) -> Value {
        let mut stream = UnixStream::connect(&self.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        write!(stream, "POST /v1/sessions/{id}/stop HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{{}}").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
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

    fn wait_file(&self, name: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = fs::read_to_string(self.home.path().join(name))
                && !text.is_empty()
            {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "The child did not create {name}."
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_group_gone(&self, id: &str) {
        let entries = self.entries(id);
        let pgid = entries
            .iter()
            .find(|e| e.event_type == "session.running")
            .unwrap()
            .body["process"]["pgid"]
            .as_u64()
            .unwrap();
        let output = Command::new("/bin/ps")
            .args(["-axo", "pgid="])
            .output()
            .unwrap();
        assert!(output.status.success());
        let groups = String::from_utf8(output.stdout).unwrap();
        assert!(
            !groups.lines().any(|line| line.trim() == pgid.to_string()),
            "The process group {pgid} still exists."
        );
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.daemon.child.kill();
        let _ = self.daemon.child.wait();
        if let Ok(dirs) = fs::read_dir(self.home.path().join("state/sessions")) {
            for dir in dirs.flatten() {
                let id = dir.file_name().to_string_lossy().into_owned();
                for entry in self.entries(&id) {
                    if entry.event_type == "session.running"
                        && let Some(pid) = entry.body["process"]["pgid"].as_u64()
                        && pid > 1
                    {
                        let _ = Command::new("/bin/kill")
                            .args(["-KILL", &format!("-{pid}")])
                            .output();
                    }
                }
            }
        }
    }
}
