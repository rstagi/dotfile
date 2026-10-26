use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

#[allow(dead_code)]
mod common;

struct Sandbox {
    daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
}

#[test]
fn run_and_tail_follow_wait_for_the_process_exit_after_done() {
    let sandbox = Sandbox::new();
    let gate = sandbox.fifo("gate");
    let script = format!(
        "printf 'before\\n'; \"$WS_BIN\" done >/dev/null; read line < '{gate}'; printf 'after\\n'; exit 7",
        gate = gate.display()
    );
    let mut run = sandbox
        .command()
        .args(["session", "run", "--headless"])
        .arg(format!("--env=WS_BIN={}", env!("CARGO_BIN_EXE_ws")))
        .args(["--", "/bin/sh", "-c", &script])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let id = sandbox.wait_for(|sessions| {
        sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["state"] == "finished")
            .map(|session| session["id"].as_str().unwrap().to_owned())
    });
    let mut tail = sandbox
        .command()
        .args(["session", "tail", &id, "--follow"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(tail.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    assert_eq!(first, "before\n");
    fs::OpenOptions::new()
        .write(true)
        .open(&gate)
        .unwrap()
        .write_all(b"continue\n")
        .unwrap();
    let mut remaining = String::new();
    reader.read_to_string(&mut remaining).unwrap();
    assert!(tail.wait().unwrap().success());
    assert_eq!(remaining, "after\n");
    assert_eq!(run.wait().unwrap().code(), Some(7));
    let session = sandbox.get(&id);
    assert_eq!(session["state"], "finished");
    assert_eq!(session["exit_code"], 7);
}

#[test]
fn phase_worktree_survives_done_and_is_removed_after_the_process_exit() {
    let sandbox = Sandbox::new();
    let repo = sandbox.home.path().join("repo");
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
    let (first, second) = (sandbox.fifo("first"), sandbox.fifo("second"));
    let script = format!(
        "read line < '{first}'; \"$WS_BIN\" done >/dev/null; read line < '{second}'; exit 7",
        first = first.display(),
        second = second.display()
    );
    let id = sandbox.create(&[
        &format!("--env=WS_BIN={}", env!("CARGO_BIN_EXE_ws")),
        "--",
        "/bin/sh",
        "-c",
        &script,
    ]);
    let output = sandbox
        .command()
        .env("WS_SESSION_ID", &id)
        .args(["phase", "worktree", repo.to_str().unwrap(), "HEAD"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    fs::write(&first, "continue\n").unwrap();
    sandbox.wait_for(|_| (sandbox.get(&id)["state"] == "finished").then_some(()));
    assert!(path.exists());
    fs::write(&second, "continue\n").unwrap();
    let waited = sandbox
        .command()
        .args(["wait", &id, "--for", "event=process.exited"])
        .output()
        .unwrap();
    assert!(waited.status.success(), "{waited:?}");
    assert_eq!(sandbox.get(&id)["exit_code"], 7);
    assert!(!path.exists());
    assert!(!git(&repo, &["worktree", "list", "--porcelain"]).contains(path.to_str().unwrap()));
}

#[test]
fn restart_keeps_supervising_a_group_whose_leader_exited() {
    let mut sandbox = Sandbox::new();
    let leader_file = sandbox.home.path().join("leader");
    let child_file = sandbox.home.path().join("child");
    let script = format!(
        "sleep 120 & echo $! > '{child}.tmp'; mv '{child}.tmp' '{child}'; echo $$ > '{leader}.tmp'; mv '{leader}.tmp' '{leader}'",
        child = child_file.display(),
        leader = leader_file.display()
    );
    let id = sandbox.create(&["--", "/bin/sh", "-c", &script]);
    let leader = sandbox.wait_for(|_| read_pid(&leader_file));
    let child = read_pid(&child_file).unwrap();
    sandbox.wait_for(|_| (!alive(leader)).then_some(()));
    assert_eq!(sandbox.get(&id)["state"], "running");
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    sandbox.daemon = common::Daemon::start(
        common::ws(sandbox.home.path(), &sandbox.socket),
        &sandbox.socket,
    );
    let session = sandbox.get(&id);
    assert_eq!(session["state"], "running", "{session}");
    assert!(alive(child));
    let stop = sandbox
        .command()
        .args(["session", "stop", &id])
        .output()
        .unwrap();
    assert!(stop.status.success(), "{stop:?}");
    assert!(!alive(child));
    assert_eq!(sandbox.get(&id)["state"], "finished");
}

#[test]
fn a_requested_stop_finishes_the_session_and_an_unrequested_signal_fails_it() {
    let sandbox = Sandbox::new();
    let stopped = sandbox.create(&["--", "/bin/sh", "-c", "exec sleep 120"]);
    sandbox.wait_for(|_| (sandbox.get(&stopped)["state"] == "running").then_some(()));
    let stop = sandbox
        .command()
        .args(["session", "stop", &stopped])
        .output()
        .unwrap();
    assert!(stop.status.success(), "{stop:?}");
    let session = sandbox.get(&stopped);
    assert_eq!(session["state"], "finished", "{session}");
    assert_eq!(session["exit_code"], 143, "{session}");
    let killed = sandbox.create(&["--", "/bin/sh", "-c", "kill -TERM $$; sleep 120"]);
    sandbox.wait_for(|_| (sandbox.get(&killed)["state"] == "failed").then_some(()));
    assert_eq!(sandbox.get(&killed)["exit_code"], 143);
}

#[test]
fn a_session_that_published_done_stays_finished_after_a_stop() {
    let sandbox = Sandbox::new();
    let script = "\"$WS_BIN\" done >/dev/null; exec sleep 120";
    let id = sandbox.create(&[
        &format!("--env=WS_BIN={}", env!("CARGO_BIN_EXE_ws")),
        "--",
        "/bin/sh",
        "-c",
        script,
    ]);
    sandbox.wait_for(|_| (sandbox.get(&id)["state"] == "finished").then_some(()));
    let stop = sandbox
        .command()
        .args(["session", "stop", &id])
        .output()
        .unwrap();
    assert!(stop.status.success(), "{stop:?}");
    assert_eq!(sandbox.get(&id)["state"], "finished");
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn alive(pid: u32) -> bool {
    let state = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8(state.stdout).unwrap();
    !state.trim().is_empty() && !state.trim().starts_with('Z')
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
            .env_remove("WS_SESSION")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command
    }

    fn fifo(&self, name: &str) -> PathBuf {
        let path = self.home.path().join(name);
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        path
    }

    fn create(&self, args: &[&str]) -> String {
        let output = self
            .command()
            .args(["session", "create", "--headless"])
            .args(args)
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

    fn wait_for<T>(&self, mut check: impl FnMut(&Value) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let response = common::get(&self.socket, "/v1/sessions");
            let sessions = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1)
                .unwrap_or(Value::Null);
            if let Some(value) = check(&sessions) {
                return value;
            }
            assert!(Instant::now() < deadline, "The condition did not occur.");
            thread::yield_now();
        }
    }
}
