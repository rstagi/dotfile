use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use ws_core::{ids::Ulid, log::SessionLog};

#[allow(dead_code)]
mod common;

struct Client(Child);

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
    assert_eq!(session["state"], "finished");
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

#[test]
fn run_waits_for_success_with_the_headless_io_arguments() {
    let sandbox = Sandbox::new();
    fs::create_dir(sandbox.home.path().join("work")).unwrap();
    fs::write(sandbox.home.path().join("input"), "input bytes").unwrap();
    let parent = sandbox.create("exit 0");
    let mut client = Client(
        sandbox
            .command()
            .args([
                "session",
                "run",
                "--headless",
                "--parent",
                &parent,
                "--cwd",
                "work",
                "--env",
                "VALUE=first",
                "--env",
                "VALUE=last",
                "--stdin",
                "input",
                "--stdout",
                "output",
                "--stderr",
                "error",
                "--",
                "/bin/sh",
                "-c",
                "printf '%s\\n' \"$VALUE\"; cat; printf failure >&2",
            ])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert_eq!(client.wait().code(), Some(0));
    assert_eq!(
        fs::read_to_string(sandbox.home.path().join("output")).unwrap(),
        "last\ninput bytes"
    );
    assert_eq!(
        fs::read_to_string(sandbox.home.path().join("error")).unwrap(),
        "failure"
    );
}

#[test]
fn run_returns_a_nonzero_child_exit_code() {
    let sandbox = Sandbox::new();
    let mut client = sandbox.run("printf '%s' \"$WS_SESSION_ID\" > ready; exit 7");
    assert_eq!(client.wait().code(), Some(7));
    let id = sandbox.wait_file("ready");
    sandbox.assert_exit(&id, 7);
}

#[test]
fn sigterm_to_run_stops_its_child_and_waits_for_exit() {
    cancel_run("-TERM", false);
}

#[test]
fn sigint_to_run_stops_its_child_and_waits_for_exit() {
    cancel_run("-INT", false);
}

#[test]
fn sighup_to_run_kills_an_uncooperative_child_and_waits_for_exit() {
    cancel_run("-HUP", true);
}

fn cancel_run(signal: &str, ignore: bool) {
    let sandbox = Sandbox::new();
    let trap = if ignore {
        "printf term > term"
    } else {
        "exit 23"
    };
    let mut client = sandbox.run(&format!(
        "trap '{trap}' TERM; printf '%s' \"$WS_SESSION_ID\" > ready; while :; do sleep 1; done"
    ));
    let id = sandbox.wait_file("ready");
    assert!(
        Command::new("/bin/kill")
            .args([signal, &client.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let code = if ignore { 137 } else { 23 };
    assert_eq!(client.wait().code(), Some(code));
    if ignore {
        assert_eq!(sandbox.wait_file("term"), "term");
    }
    sandbox.assert_exit(&id, code);
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

    fn run(&self, script: &str) -> Client {
        Client(
            self.command()
                .args(["session", "run", "--", "/bin/sh", "-c", script])
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    fn assert_exit(&self, id: &str, code: i32) {
        let exit = self
            .entries(id)
            .into_iter()
            .find(|entry| entry.event_type == "exit")
            .expect("The exit event must exist before the client exits.");
        assert_eq!(exit.body["code"], code);
        self.assert_group_gone(id);
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

impl Client {
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "The client did not exit.");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
