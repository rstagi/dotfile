#[path = "../../ws-daemon/src/import/fixtures/mod.rs"]
mod fixtures;

#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn dispatcher(home: &Path, socket: &Path) -> Command {
    let base = common::ws(home, socket);
    let mut command = Command::new("zsh");
    command.arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ws/ws.sh"));
    for (key, value) in base.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    for key in ["WS_SESSION", "WS_SESSION_ID", "TMUX", "TMUX_PANE"] {
        command.env_remove(key);
    }
    command.env("WS_RUST_BIN", env!("CARGO_BIN_EXE_ws"));
    command.env("WS_TMUX_SOCKET", home.join("tmux.sock"));
    command
}

#[tokio::test]
async fn dispatcher_smoke_covers_rust_sessions_messages_wait_and_mcp() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
    use std::time::Duration;

    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let mut command = common::ws(home.path(), &socket);
    command.env("WS_TMUX_SOCKET", home.path().join("tmux.sock"));
    let _daemon = common::Daemon::start(command, &socket);
    let output = dispatcher(home.path(), &socket)
        .args(["backend", "rust"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["ping"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["session", "create", "--name", "reader", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let session: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = session["id"].as_str().unwrap();
    for variable in ["WS_SESSION_ID", "WS_SESSION"] {
        let output = dispatcher(home.path(), &socket)
            .env(variable, id)
            .args(["whoami", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"],
            id
        );
    }
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["name", "renamed"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    for target in ["#1", "renamed", id] {
        let output = dispatcher(home.path(), &socket)
            .args(["show", target, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"],
            id
        );
    }
    let output = dispatcher(home.path(), &socket)
        .args(["send", "renamed", "smoke message"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["inbox", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox[0]["body"], "smoke message");
    let event_id = inbox[0]["event_id"].as_str().unwrap();
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["ack", event_id])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["inbox", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([])
    );
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["done"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["wait", id, "--for", "done", "--timeout", "30"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 2);
    let mut command = tokio::process::Command::from(dispatcher(home.path(), &socket));
    command.arg("mcp").env("WS_SESSION_ID", id);
    let client = tokio::time::timeout(
        Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("session_list")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text = result.content[0].as_text().unwrap();
    let listed: Value = serde_json::from_str(&text.text).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[0]["id"], id);
    assert_eq!(listed[0]["state"], "finished");
    client.cancel().await.unwrap();
}

#[test]
fn zsh_backend_is_rejected() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let mut command = dispatcher(home.path(), &socket);
    let output = command.args(["backend", "zsh"]).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Only the rust backend is available."),
        "{output:?}"
    );
    assert_eq!(
        fs::read_to_string(home.path().join(".ws/backend")).unwrap(),
        "rust\n"
    );
    let response = common::request(
        &socket, "POST", "/v1/backend",
        &json!({"backend":"zsh", "home":home.path().join(".ws"), "root":home.path().join(".ws/worktrees")}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
}

#[test]
fn dispatcher_ignores_a_retired_zsh_selector() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let mut command = dispatcher(home.path(), &socket);
    fs::write(home.path().join(".ws/backend"), "zsh\n").unwrap();
    let output = command.args(["--help"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("daemon"),
        "{output:?}"
    );
}

#[test]
fn absent_selector_dispatches_to_rust() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let mut command = dispatcher(home.path(), &socket);
    fs::remove_file(home.path().join(".ws/backend")).unwrap();
    let output = command.args(["--help"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("daemon"),
        "{output:?}"
    );
    assert!(!home.path().join(".ws/backend").exists());
}

#[test]
fn existing_rust_sessions_prevent_an_automatic_legacy_import() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let state = home.path().join("state");
    let mut sessions = ws_daemon::sessions::Sessions::open(&state, 1).unwrap();
    let session = sessions.create(Default::default()).unwrap();
    drop(sessions);
    let legacy = home.path().join(".ws/sessions/2");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("meta"), "invalid legacy metadata").unwrap();
    let mut command = common::ws(home.path(), &socket);
    command.env("WS_TMUX_SOCKET", home.path().join("tmux.sock"));
    let _daemon = common::Daemon::start(command, &socket);
    let output = dispatcher(home.path(), &socket)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["id"], session.id.to_string());
    assert!(!state.join("import-zsh.json").exists());
}

#[test]
fn first_start_imports_legacy_state_once_before_serving_commands() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let legacy = home.path().join(".ws");
    let mut command = common::ws(home.path(), &socket);
    command.env("WS_TMUX_SOCKET", home.path().join("tmux.sock"));
    fs::remove_file(legacy.join("backend")).unwrap();
    fixtures::copy("exit", &legacy);
    fs::write(
        legacy.join("sessions/2/inbox"),
        "{\"message\":\"legacy message\"}\n",
    )
    .unwrap();
    let workspace = legacy.join("worktrees/project/main");
    fs::create_dir_all(&workspace).unwrap();
    for args in [
        vec!["init", "-qb", "main"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "initial",
            "--allow-empty",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    fs::write(legacy.join("pins"), "project/main\n").unwrap();
    let original = fs::read(legacy.join("sessions/2/inbox")).unwrap();
    let daemon = common::Daemon::start(command, &socket);
    let output = dispatcher(home.path(), &socket)
        .args(["show", "#2", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let session: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(session["alias"], 2);
    let id = session["id"].as_str().unwrap();
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["inbox", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["body"], "legacy message");
    let output = dispatcher(home.path(), &socket)
        .args(["triggers", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let triggers: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(triggers.as_array().unwrap().len(), 1);
    let output = dispatcher(home.path(), &socket)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let workspaces: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(workspaces[0]["id"], "project/main");
    assert_eq!(workspaces[0]["body"]["pinned"], true);
    assert_eq!(fs::read(legacy.join("sessions/2/inbox")).unwrap(), original);
    let journal = fs::read(home.path().join("state/import-zsh.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&journal).unwrap()["complete"],
        true
    );
    drop(daemon);
    // A crash after the appends but before the completion marker must not duplicate messages.
    let mut incomplete: Value = serde_json::from_slice(&journal).unwrap();
    incomplete["complete"] = json!(false);
    fs::write(
        home.path().join("state/import-zsh.json"),
        incomplete.to_string(),
    )
    .unwrap();
    let mut command = common::ws(home.path(), &socket);
    command.env("WS_TMUX_SOCKET", home.path().join("tmux.sock"));
    let daemon = common::Daemon::start(command, &socket);
    let output = dispatcher(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["inbox", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        inbox
    );
    let resumed = fs::read(home.path().join("state/import-zsh.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&resumed).unwrap()["complete"],
        true
    );
    drop(daemon);
    // A completed startup import does not read the source again.
    fs::write(legacy.join("sessions/2/meta"), "invalid after import").unwrap();
    let mut command = common::ws(home.path(), &socket);
    command.env("WS_TMUX_SOCKET", home.path().join("tmux.sock"));
    let _daemon = common::Daemon::start(command, &socket);
    let output = dispatcher(home.path(), &socket)
        .args(["show", "#2", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let restored: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(restored["id"], session["id"]);
    assert_eq!(
        fs::read(home.path().join("state/import-zsh.json")).unwrap(),
        resumed
    );
}

#[test]
fn launcher_dispatches_internal_commands_without_backend_environment_variables() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let directory = home.path().join(".ws");
    fs::create_dir_all(directory.join("bin")).unwrap();
    fs::write(directory.join("backend"), "zsh\n").unwrap();
    let binary = directory.join("bin/ws-rs");
    fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    for command in [
        "_agent",
        "_end",
        "_deliver-retry",
        "_evaluate",
        "_hook-retry",
    ] {
        let output = Command::new("zsh")
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("WS_BACKEND", "zsh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ws/ws.sh"))
            .args([command, "literal argument"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{command}\nliteral argument\n")
        );
    }
}

#[test]
fn rust_allocation_waits_for_the_zsh_counter_lock() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
        sync::mpsc,
        time::{Duration, Instant},
    };
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let response = common::request(&socket, "POST", "/v1/sessions", "{}");
    let parent: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let mut holder = ChildGuard(
        Command::new("zsh")
            .args([
                "-fc",
                r#"zmodload zsh/system; zsystem flock "$1" || exit; print ready; read -r release"#,
                "counter-holder",
            ])
            .arg(home.path().join(".ws/sessions/counter.lock"))
            .env("HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = holder.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let _ = ready_tx.send(line);
    });
    assert_eq!(
        ready_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        "ready\n"
    );
    let (tx, rx) = mpsc::channel();
    let request_socket = socket.clone();
    let request = json!({"parent_session_id":parent["id"]}).to_string();
    std::thread::spawn(move || {
        let _ = tx.send(common::request(
            &request_socket,
            "POST",
            "/v1/sessions",
            &request,
        ));
    });
    let log = ws_core::log::SessionLog::new(
        &home.path().join("state"),
        parent["id"].as_str().unwrap().parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !log.read().unwrap().entries.iter().any(|entry| {
        entry.event_type == "api.called" && entry.body["operation"] == "session.create"
    }) {
        assert!(
            Instant::now() < deadline,
            "The allocation request did not reach the daemon."
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(
        fs::read_to_string(home.path().join(".ws/sessions/counter")).unwrap(),
        "1\n"
    );
    writeln!(holder.0.stdin.take().unwrap(), "release").unwrap();
    let response = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let session: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(session["alias"], 2);
    assert_eq!(
        fs::read_to_string(home.path().join(".ws/sessions/counter")).unwrap(),
        "2\n"
    );
}
