use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
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
fn inherited_environment_reaches_the_child_without_entering_durable_state() {
    let mut sandbox = Sandbox::new();
    let marker = Ulid::new().to_string();
    let value = format!("{marker} spaces=and\nnewlines");
    let output = sandbox
        .command()
        .env("WS_TEST_INHERITED", &value)
        .env("WS_TEST_EXPECTED", &value)
        .env("WS_TEST_OVERRIDE", &value)
        .args([
            "session", "run", "--inherit-env",
            "--env", "WS_TEST_OVERRIDE=explicit",
            "--", "/bin/sh", "-c",
            "test -n \"$WS_TEST_INHERITED\" && test \"$WS_TEST_INHERITED\" = \"$WS_TEST_EXPECTED\" && test \"$WS_TEST_OVERRIDE\" = explicit",
        ])
        .output();
    let _ = sandbox.daemon.child.kill();
    let _ = sandbox.daemon.child.wait();
    let daemon_errors = sandbox.daemon.stderr();
    // Drop before assertions so a failure cannot print daemon environment values.
    drop(sandbox.daemon);
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "The child did not receive the expected environment."
    );
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&marker));
    assert!(!daemon_errors.contains(&marker));
    assert_directory_has_no_marker(sandbox.home.path(), marker.as_bytes());
}

#[test]
#[cfg(target_os = "macos")]
fn agent_priority_clamps_headless_commands() {
    let inherited_clamp = common::test_process_has_inherited_clamp();
    // macOS reports a background clamp as scheduling priority 4.
    for (config, clamped) in [
        ("[agent]\npriority = \"background\"\n", true),
        ("[agent]\npriority = \"normal\"\n", false),
    ] {
        let sandbox = Sandbox::with_config(config);
        let id = sandbox.create(&["--", "/bin/sh", "-c", "ps -o pri= -p \"$$\""]);
        assert_eq!(sandbox.wait(&id)["exit_code"], 0);
        let stdout = fs::read_to_string(
            sandbox
                .home
                .path()
                .join("state/sessions")
                .join(&id)
                .join("stdout.log"),
        )
        .unwrap();
        if !inherited_clamp {
            assert_eq!(stdout.trim() == "4", clamped, "{config}: {stdout}");
        }
    }
}

#[test]
fn headless_command_has_a_process_group_separate_logs_and_a_durable_successful_exit() {
    let mut sandbox = Sandbox::new();
    let id = sandbox.create(&[
        "--",
        "/bin/sh",
        "-c",
        "printf '%s ' \"$$\"; ps -o pgid= -p \"$$\"; printf 'failure stream\\n' >&2",
    ]);
    let session = sandbox.wait(&id);
    assert_eq!(session["state"], "finished");
    assert_eq!(session["exit_code"], 0);
    assert_eq!(session["body"]["kind"], "headless");
    let directory = sandbox.home.path().join("state/sessions").join(&id);
    let stdout = fs::read_to_string(directory.join("stdout.log")).unwrap();
    let ids: Vec<_> = stdout.split_whitespace().collect();
    assert_eq!(ids.len(), 2, "{stdout}");
    assert_eq!(ids[0], ids[1], "The command must lead its process group.");
    assert_eq!(
        fs::read_to_string(directory.join("stderr.log")).unwrap(),
        "failure stream\n"
    );
    assert_eq!(
        sandbox.output(&["session", "tail", &id]).stdout,
        stdout.as_bytes()
    );
    let elsewhere = sandbox
        .command()
        .current_dir("/tmp")
        .args(["session", "tail", &id])
        .output()
        .unwrap();
    assert!(elsewhere.status.success(), "{elsewhere:?}");
    assert_eq!(elsewhere.stdout, stdout.as_bytes());
    assert_eq!(
        sandbox
            .output(&["session", "tail", &id, "--stream", "stderr"])
            .stdout,
        b"failure stream\n"
    );
    let log = SessionLog::new(
        &sandbox.home.path().join("state"),
        id.parse().unwrap(),
        Ulid::new(),
    );
    let entries = log.read().unwrap().entries;
    assert_eq!(
        entries
            .iter()
            .map(|e| e.event_type.as_str())
            .collect::<Vec<_>>(),
        [
            "session.created",
            "session.running",
            "exit",
            "session.finished",
            "process.exited"
        ]
    );
    assert_eq!(entries[2].body["code"], 0);
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox.daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.get(&id)["exit_code"], 0);
}

#[test]
fn headless_flags_control_parent_cwd_environment_stdin_and_both_output_files() {
    let sandbox = Sandbox::new();
    let parent = sandbox.create(&["--", "/usr/bin/true"]);
    sandbox.wait(&parent);
    let cwd = sandbox.home.path().join("working directory");
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("input"), "input bytes\n").unwrap();
    let id = sandbox.create(&[
        "--parent", "#1", "--cwd", cwd.to_str().unwrap(),
        "--env", "HEADLESS_A=first", "--env", "HEADLESS_A=replaced=value",
        "--env", "HEADLESS_B=", "--stdin", "working directory/input",
        "--stdout", "custom output", "--stderr", "custom error",
        "--", "/bin/sh", "-c",
        "pwd; printf '%s|%s|%s\\n' \"$HEADLESS_A\" \"$HEADLESS_B\" \"$WS_SESSION_ID\"; cat; printf 'custom error\\n' >&2; exit 7",
    ]);
    let session = sandbox.wait(&id);
    assert_eq!(session["state"], "failed");
    assert_eq!(session["exit_code"], 7);
    assert_eq!(session["parent_session_id"], parent);
    let expected = format!(
        "{}\nreplaced=value||{id}\ninput bytes\n",
        cwd.canonicalize().unwrap().display()
    );
    assert_eq!(
        fs::read(sandbox.home.path().join("custom output")).unwrap(),
        expected.as_bytes()
    );
    assert_eq!(
        fs::read(sandbox.home.path().join("custom error")).unwrap(),
        b"custom error\n"
    );
    assert_eq!(
        sandbox.output(&["session", "tail", &id]).stdout,
        expected.as_bytes()
    );
    assert_eq!(
        sandbox
            .output(&["session", "tail", &id, "--stream", "stderr"])
            .stdout,
        b"custom error\n"
    );
    let entries = SessionLog::new(
        &sandbox.home.path().join("state"),
        id.parse().unwrap(),
        Ulid::new(),
    )
    .read()
    .unwrap()
    .entries;
    assert_eq!(entries[2].event_type, "exit");
    assert_eq!(entries[2].body["code"], 7);
    assert_eq!(entries[3].event_type, "session.failed");
    assert!(
        entries
            .iter()
            .all(|e| e.body["parent_session_id"] == parent)
    );
}

#[test]
fn headless_commands_remove_the_legacy_identity_from_the_launch_environment() {
    let sandbox = Sandbox::new();
    let id = sandbox.create(&[
        "--env",
        "WS_SESSION=legacy-parent",
        "--",
        "/bin/sh",
        "-c",
        "printf '%s|%s' \"$WS_SESSION_ID\" \"${WS_SESSION-unset}\"",
    ]);
    sandbox.wait(&id);
    assert_eq!(
        sandbox.output(&["session", "tail", &id]).stdout,
        format!("{id}|unset").as_bytes()
    );
}

#[test]
fn headless_agents_use_the_shared_preparation_contract() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    for agent in ["claude", "codex", "grok"] {
        let executable = sandbox.home.path().join("fake-agents").join(agent);
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s\\n' \"$WS_SESSION_ID\" \"$WS_SESSION\" \"$@\"\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let id = sandbox.create(&["--", agent, "prompt"]);
        sandbox.wait(&id);
        let bytes = sandbox.output(&["session", "tail", &id]).stdout;
        let text = String::from_utf8(bytes).unwrap();
        let arguments: Vec<_> = text.lines().collect();
        assert_eq!(arguments[..2], [id.as_str(), ""]);
        match agent {
            "claude" => {
                assert!(arguments.contains(&"-p"), "{text}");
                let settings =
                    arguments[arguments.iter().position(|v| *v == "--settings").unwrap() + 1];
                let hooks: Value = serde_json::from_slice(&fs::read(settings).unwrap()).unwrap();
                assert!(hooks["hooks"]["Stop"].is_array());
            }
            "codex" => {
                assert!(arguments.contains(&"exec"), "{text}");
                assert!(
                    arguments.iter().any(|s| s.starts_with("hooks.Stop=")),
                    "{text}"
                );
            }
            _ => assert!(sandbox.home.path().join(".grok/hooks/ws.json").is_file()),
        }
    }
}

#[test]
fn headless_rejects_overlapping_io_files_without_destroying_existing_contents() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.home.path().join("saved"), "keep this input").unwrap();
    fs::hard_link(
        sandbox.home.path().join("saved"),
        sandbox.home.path().join("same file"),
    )
    .unwrap();
    for args in [
        vec!["--stdout", "saved", "--stderr", "same file"],
        vec!["--stdin", "saved", "--stdout", "same file"],
    ] {
        let output = sandbox
            .command()
            .args(["session", "create", "--headless"])
            .args(args)
            .args(["--", "/usr/bin/true"])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "Overlapping streams must fail: {output:?}"
        );
        assert_eq!(
            fs::read_to_string(sandbox.home.path().join("saved")).unwrap(),
            "keep this input"
        );
    }
}

#[test]
fn tail_follow_streams_each_output_before_exit_and_retains_binary_bytes() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        process::Stdio,
        sync::mpsc,
    };
    let sandbox = Sandbox::new();
    for stream in ["stdout", "stderr"] {
        let gate = sandbox.home.path().join(format!("gate-{stream}"));
        assert!(
            Command::new("mkfifo")
                .arg(&gate)
                .status()
                .unwrap()
                .success()
        );
        let script = format!(
            "printf 'early\\n' {redirect}; read line < '{gate}'; printf 'late\\000bytes\\n' {redirect}",
            redirect = if stream == "stderr" { ">&2" } else { "" },
            gate = gate.display()
        );
        let id = sandbox.create(&["--", "/bin/sh", "-c", &script]);
        let mut tail = sandbox
            .command()
            .args(["session", "tail", &id, "--follow", "--stream", stream])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(tail.stdout.take().unwrap());
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            sender.send(first).unwrap();
            let mut remaining = Vec::new();
            reader.read_to_end(&mut remaining).unwrap();
            remaining
        });
        let first = receiver.recv_timeout(Duration::from_secs(5));
        let before_exit = sandbox.get(&id);
        fs::OpenOptions::new()
            .write(true)
            .open(&gate)
            .unwrap()
            .write_all(b"continue\n")
            .unwrap();
        sandbox.wait(&id);
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = tail.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                tail.kill().unwrap();
                tail.wait().unwrap();
                panic!("The tail command did not finish after the session exited.");
            }
            thread::yield_now();
        };
        let remaining = reader.join().unwrap();
        assert_eq!(first.unwrap(), "early\n");
        assert_eq!(before_exit["state"], "running");
        assert!(status.success());
        assert_eq!(remaining, b"late\0bytes\n");
    }
}

#[test]
fn headless_defaults_stdin_to_eof_and_reports_a_signal_as_a_failed_exit() {
    let sandbox = Sandbox::new();
    let id = sandbox.create(&["--", "/bin/cat"]);
    assert_eq!(sandbox.wait(&id)["exit_code"], 0);
    assert!(sandbox.output(&["session", "tail", &id]).stdout.is_empty());
    let id = sandbox.create(&["--", "/bin/sh", "-c", "kill -TERM $$"]);
    let session = sandbox.wait(&id);
    assert_eq!(session["state"], "failed");
    assert_eq!(session["exit_code"], 143);
    let entries = SessionLog::new(
        &sandbox.home.path().join("state"),
        id.parse().unwrap(),
        Ulid::new(),
    )
    .read()
    .unwrap()
    .entries;
    assert_eq!(entries[2].body["signal"], 15);
}

fn assert_directory_has_no_marker(directory: &std::path::Path, marker: &[u8]) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_directory_has_no_marker(&path, marker);
        } else if path.is_file() {
            let bytes = fs::read(&path).unwrap();
            assert!(
                !bytes.windows(marker.len()).any(|part| part == marker),
                "An environment value entered a sandbox file."
            );
        }
    }
}

impl Sandbox {
    fn new() -> Self {
        Self::with_config("")
    }

    fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        fs::create_dir(home.path().join("state")).unwrap();
        fs::write(home.path().join("state/config.toml"), config).unwrap();
        let socket = home.path().join("wsd.sock");
        let mut command = common::ws(home.path(), &socket);
        command.current_dir(home.path()).env("WS_STATE", "state");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            home,
            socket,
            daemon,
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

    fn output(&self, args: &[&str]) -> Output {
        let output = self.command().args(args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output
    }

    fn create(&self, args: &[&str]) -> String {
        let mut arguments = vec!["session", "create", "--headless"];
        arguments.extend_from_slice(args);
        String::from_utf8(self.output(&arguments).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn get(&self, id: &str) -> Value {
        let response = common::get(&self.socket, &format!("/v1/sessions/{id}"));
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn wait(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let session = self.get(id);
            if ["finished", "failed"].contains(&session["state"].as_str().unwrap()) {
                return session;
            }
            assert!(
                Instant::now() < deadline,
                "The command did not exit: {session}"
            );
            thread::yield_now();
        }
    }
}

#[test]
fn supervised_agents_override_inherited_identity_and_privacy_settings() {
    use std::os::unix::fs::PermissionsExt;
    use ws_core::{device::Device, semconv};
    let sandbox = Sandbox::new();
    let device = Device::load(&sandbox.home.path().join("state")).unwrap();
    for agent in ["claude", "codex"] {
        let executable = sandbox.home.path().join("fake-agents").join(agent);
        fs::write(&executable, r#"#!/usr/bin/python3
import json, os, sys
if sys.argv[1:] == ['--version']:
    print('test-agent 0.0.0')
    sys.exit(0)
print(json.dumps(dict(resource=os.environ['OTEL_RESOURCE_ATTRIBUTES'], native=os.getenv('TRACEPARENT'), content=os.getenv('OTEL_LOG_USER_PROMPTS'))))
"#).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let id = sandbox.create(&[
            "--env",
            "OTEL_RESOURCE_ATTRIBUTES=wrong-session",
            "--env",
            "TRACEPARENT=wrong-parent",
            "--env",
            "OTEL_LOG_USER_PROMPTS=1",
            "--",
            agent,
            "prompt",
        ]);
        sandbox.wait(&id);
        let captured: Value =
            serde_json::from_slice(&sandbox.output(&["session", "tail", &id]).stdout).unwrap();
        let resources: std::collections::BTreeMap<_, _> = captured["resource"]
            .as_str()
            .unwrap()
            .split(',')
            .map(|part| part.split_once('=').unwrap())
            .collect();
        assert_eq!(resources[semconv::SESSION_ID], id);
        assert_eq!(resources[semconv::DEVICE_ID], device.device_id.to_string());
        assert!(captured["native"].is_null());
        assert!(captured["content"].is_null());
    }
}
