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
    command
}

#[test]
fn selector_dispatches_internal_commands_without_backend_environment_variables() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let directory = home.path().join(".ws");
    fs::create_dir_all(directory.join("bin")).unwrap();
    fs::write(directory.join("backend"), "rust\n").unwrap();
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
fn switches_backends_and_imports_new_zsh_sessions_without_losing_rust_messages() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let output = dispatcher(home.path(), &socket)
        .args(["backend", "rust"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(home.path().join(".ws/backend")).unwrap(),
        "rust\n"
    );
    let response = common::request(
        &socket,
        "POST",
        "/v1/sessions",
        &json!({"name":"first"}).to_string(),
    );
    let first: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(first["alias"], 1);
    let output = dispatcher(home.path(), &socket)
        .args(["send", "first", "retained Rust message"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["backend", "zsh"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("2 Rust-only sessions"),
        "{output:?}"
    );
    let legacy = Command::new("zsh")
        .args([
            "-fc",
            r#"
        source "$1/ws/ws.sh"
        workspace_name() { print -r -- repo/main; }
        id=$(ws_bus_allocate /sandbox 1 codex '') || exit
        with_lock "$WS_SESSIONS/$id/inbox" ws_bus_enqueue "$id" 'created under zsh'
        print -r -- "$id"
    "#,
            "legacy",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
        .env("HOME", home.path())
        .env("WS_HOME", home.path().join(".ws"))
        .output()
        .unwrap();
    assert!(legacy.status.success(), "{legacy:?}");
    assert_eq!(String::from_utf8_lossy(&legacy.stdout).trim(), "3");
    let output = dispatcher(home.path(), &socket)
        .args(["backend", "rust"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = dispatcher(home.path(), &socket)
        .args(["sessions", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 3);
    for (target, expected) in [
        (first["id"].as_str().unwrap(), "retained Rust message"),
        ("#3", "created under zsh"),
    ] {
        let response = common::get(
            &socket,
            &format!("/v1/sessions/{}", target.replace('#', "%23")),
        );
        let session: Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let output = dispatcher(home.path(), &socket)
            .env("WS_SESSION_ID", session["id"].as_str().unwrap())
            .args(["inbox", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let inbox: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(inbox.as_array().unwrap().len(), 1);
        assert_eq!(inbox[0]["body"], expected);
    }
    let output = dispatcher(home.path(), &socket)
        .args(["backend", "rust"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
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
