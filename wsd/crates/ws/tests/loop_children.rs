use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
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

struct TestProcess(Option<u32>);

impl Drop for TestProcess {
    fn drop(&mut self) {
        let Some(pid) = self.0 else { return };
        // The script execs sleep, so the recorded child has no descendants.
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .output();
    }
}

#[test]
fn loop_review_events_copy_metadata_and_keyed_retries_append_once() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let child = sandbox.create(&review(&owner, "r1"), "true");
    sandbox.wait_state(&child, "finished");
    let retry = sandbox.create(&review(&owner, "r1"), "true");
    assert_eq!(retry, child);
    let events = sandbox.loop_events(&owner, "loop.review");
    assert_eq!(events.len(), 2);
    for (event, state) in events.iter().zip(["started", "exited"]) {
        assert_eq!(event["state"], state);
        assert_eq!(event["phase"], 2);
        assert_eq!(event["attempt"], 1);
        assert_eq!(event["repo"], "owner/repo");
        assert_eq!(event["role"], "review");
        assert_eq!(event["provider"], "gpt");
        assert_eq!(event["round"], 1);
        assert_eq!(event["inner"], 1);
        assert_eq!(event["key"], "r1");
        assert_eq!(event["session"], child);
        assert_eq!(event["source"], "daemon");
        assert!(event.get("verdict").is_none());
    }
    let next = sandbox.create(&review(&owner, "r2"), "true");
    sandbox.wait_state(&next, "finished");
    assert_ne!(next, child);
    let events = sandbox.loop_events(&owner, "loop.review");
    assert_eq!(events.len(), 4);
    assert_eq!(events[2]["state"], "started");
    assert_eq!(events[2]["key"], "r2");
    assert_eq!(events[2]["session"], next);
}

#[test]
fn loop_attempt_exit_is_recorded_once_after_a_daemon_restart() {
    let mut sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    let flags = [
        "--loop",
        &owner,
        "--phase",
        "117",
        "--attempt",
        "1",
        "--key",
        "p117-a1",
    ];
    let child = sandbox.create(&flags, &sandbox.script("ready", "exec /bin/sleep 120"));
    let running = sandbox.entry(&child, "session.running");
    let mut group = TestProcess(Some(running.body["process"]["pid"].as_u64().unwrap() as u32));
    sandbox.wait_until(|| sandbox.lines("ready") == 1);
    let events = sandbox.loop_events(&owner, "loop.attempt");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["state"], "started");
    sandbox.restart();
    assert_eq!(sandbox.create(&flags, "true"), child);
    sandbox.run_ok(sandbox.command().args(["session", "stop", &child]));
    group.0 = None;
    sandbox.restart();
    let events = sandbox.loop_events(&owner, "loop.attempt");
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1]["state"], "exited");
    // The daemon cannot recover a process status that belonged to its predecessor.
    assert_eq!(events[1]["exit"], -1);
    for event in &events {
        assert_eq!(event["n"], 117);
        assert_eq!(event["attempt"], 1);
        assert_eq!(event["key"], "p117-a1");
        assert_eq!(event["session"], child);
        assert_eq!(event["source"], "daemon");
        assert!(event.get("verdict").is_none());
    }
    let next = sandbox.create(
        &[
            "--loop",
            &owner,
            "--phase",
            "117",
            "--attempt",
            "2",
            "--key",
            "p117-a2",
        ],
        "true",
    );
    sandbox.wait_state(&next, "finished");
    let events = sandbox.loop_events(&owner, "loop.attempt");
    assert_eq!(events.len(), 4);
    assert_eq!(events[2]["state"], "started");
    assert_eq!(events[2]["attempt"], 2);
    assert_eq!(events[2]["session"], next);
    assert_eq!(events[3]["exit"], 0);
}

#[test]
fn a_new_orchestrator_gets_the_same_review_step_and_no_second_process_starts() {
    let sandbox = Sandbox::new();
    let loop_id = sandbox.loop_session();
    let first_orchestrator = sandbox.create(&[], "true");
    let script = sandbox.script("run", "sleep 120");
    let step = sandbox.create_as(&first_orchestrator, &review(&loop_id, "r1"), &script);
    sandbox.wait_until(|| sandbox.lines("run") == 1);
    let created = sandbox.entry(&step, "session.created");
    assert_eq!(created.body["parent_session_id"], loop_id);
    assert_eq!(created.body["actor"], first_orchestrator);
    assert_eq!(created.body["loop"]["role"], "review");
    assert_eq!(created.body["loop"]["loop"], loop_id);
    // The first orchestrator stops while the step runs.
    sandbox.run_ok(
        sandbox
            .command()
            .args(["session", "stop", &first_orchestrator]),
    );
    assert_eq!(sandbox.get(&step)["state"], "running");
    let second_orchestrator = sandbox.create(&[], "true");
    let again = sandbox.create_as(&second_orchestrator, &review(&loop_id, "r1"), &script);
    assert_eq!(again, step);
    assert_eq!(sandbox.lines("run"), 1);
    assert_eq!(sandbox.entries(&step, "session.created"), 1);
    sandbox.run_ok(sandbox.command().args(["session", "stop", &step]));
}

#[test]
fn a_nine_step_review_runs_through_an_orchestrator_with_the_default_limits() {
    let sandbox = Sandbox::new();
    let loop_id = sandbox.loop_session();
    let orchestrator = sandbox.create(&[], "true");
    // Six reviewer and critic steps, two fixes and one final step, as in tests/ar.test.sh.
    for (index, role) in [
        "review", "critic", "review", "critic", "review", "critic", "fix", "fix", "final",
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("s{index}");
        let flags = review_step(&loop_id, &key, role);
        let step = sandbox.create_as(&orchestrator, &flags, "true");
        sandbox.wait_state(&step, "finished");
        assert_eq!(sandbox.get(&step)["body"]["parent_session_id"], loop_id);
    }
}

#[test]
fn a_caller_outside_a_loop_still_hits_the_limit_of_eight_children() {
    let sandbox = Sandbox::new();
    let parent = sandbox.create(&[], "true");
    for key in 0..8 {
        let key = format!("k{key}");
        sandbox.create(&["--parent", &parent, "--key", &key], "sleep 120");
    }
    let output = sandbox
        .command()
        .args(["session", "create", "--headless", "--parent", &parent])
        .args(["--", "/bin/sh", "-c", "true"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("max_children")
            || String::from_utf8_lossy(&output.stderr).contains("children"),
        "{output:?}"
    );
}

#[test]
fn a_script_command_reads_the_loop_environment_from_the_daemon() {
    let sandbox = Sandbox::new();
    let loop_id = sandbox.loop_session();
    let orchestrator = sandbox.create(&[], "true");
    let out = sandbox.home.path().join("environment");
    let script = format!(
        "printf '%s|%s|%s|%s' \"$WS_LOOP_ID\" \"$WS_LOOP_PHASE\" \"$WS_LOOP_ATTEMPT\" \"$WS_SESSION_ID\" > '{}'",
        out.display()
    );
    let step = sandbox.create_as(
        &orchestrator,
        &[
            "--loop",
            &loop_id,
            "--phase",
            "110",
            "--attempt",
            "2",
            "--key",
            "work",
        ],
        &script,
    );
    sandbox.wait_state(&step, "finished");
    assert_eq!(
        fs::read_to_string(out).unwrap(),
        format!("{loop_id}|110|2|{step}")
    );
}

#[test]
fn the_loop_object_must_name_a_loop_session_and_a_whole_review_step() {
    let sandbox = Sandbox::new();
    let loop_id = sandbox.loop_session();
    let plain = sandbox.create(&[], "true");
    let rejected = |flags: &[&str]| {
        let output = sandbox
            .command()
            .args(["session", "create", "--headless"])
            .args(flags)
            .args(["--", "/bin/sh", "-c", "true"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
    };
    rejected(&["--loop", &plain, "--phase", "1", "--attempt", "1"]);
    rejected(&[
        "--loop",
        &loop_id,
        "--phase",
        "1",
        "--attempt",
        "1",
        "--role",
        "review",
    ]);
    rejected(&["--phase", "1"]);
}

fn review<'a>(loop_id: &'a str, key: &'a str) -> Vec<&'a str> {
    vec![
        "--loop",
        loop_id,
        "--phase",
        "2",
        "--attempt",
        "1",
        "--repo",
        "owner/repo",
        "--role",
        "review",
        "--provider",
        "gpt",
        "--round",
        "1",
        "--inner",
        "1",
        "--key",
        key,
    ]
}

fn review_step<'a>(loop_id: &'a str, key: &'a str, role: &'a str) -> Vec<&'a str> {
    vec![
        "--loop",
        loop_id,
        "--phase",
        "2",
        "--attempt",
        "1",
        "--repo",
        "owner/repo",
        "--role",
        role,
        "--provider",
        "gpt",
        "--round",
        "1",
        "--inner",
        "1",
        "--key",
        key,
    ]
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        fs::create_dir(home.path().join("state")).unwrap();
        let socket = home.path().join("wsd.sock");
        let mut command = common::ws(home.path(), &socket);
        command
            .current_dir(home.path())
            .env("WS_STATE", "state")
            .env_remove("WS_LOOP_ID")
            .env_remove("WS_LOOP_PHASE")
            .env_remove("WS_LOOP_ATTEMPT");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            _daemon: daemon,
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

    fn restart(&mut self) {
        self._daemon.child.kill().unwrap();
        self._daemon.child.wait().unwrap();
        self._daemon = common::Daemon::start(self.command(), &self.socket);
    }

    fn loop_events(&self, owner: &str, kind: &str) -> Vec<Value> {
        let response = common::get(
            &self.socket,
            &format!("/v1/sessions/{owner}/log?type={kind}"),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let log: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["body"].clone())
            .collect()
    }

    fn loop_session(&self) -> String {
        let directory = self.home.path().join("loop");
        fs::create_dir_all(&directory).unwrap();
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/loops/create",
            &json!({"loop_dir":directory,"plan_id":format!("fixture:{}", directory.display())})
                .to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let body: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        body["id"].as_str().unwrap().to_owned()
    }

    fn script(&self, name: &str, tail: &str) -> String {
        format!(
            "echo x >> '{}'; {tail}",
            self.home.path().join(name).display()
        )
    }

    fn lines(&self, name: &str) -> usize {
        fs::read_to_string(self.home.path().join(name))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    fn create(&self, flags: &[&str], script: &str) -> String {
        self.create_with(self.command(), flags, script)
    }

    /// Creates a session as the caller `WS_SESSION_ID`, the way an orchestrator does.
    fn create_as(&self, caller: &str, flags: &[&str], script: &str) -> String {
        let mut command = self.command();
        command.env("WS_SESSION_ID", caller);
        self.create_with(command, flags, script)
    }

    fn create_with(&self, mut command: Command, flags: &[&str], script: &str) -> String {
        let output = command
            .args(["session", "create", "--headless"])
            .args(flags)
            .args(["--", "/bin/sh", "-c", script])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn run_ok(&self, command: &mut Command) -> Output {
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output
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
}
