#[allow(dead_code)]
mod common;

use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn command(home: &std::path::Path, socket: &std::path::Path) -> Command {
    let mut command = common::ws(home, socket);
    for key in ["WS_SESSION", "WS_SESSION_ID", "TMUX", "TMUX_PANE"] {
        command.env_remove(key);
    }
    command
}

#[test]
fn done_preserves_the_zsh_output_contract_and_wait_uses_the_daemon() {
    // Golden scenarios: ws/tests/triggers.test.sh (stdin, file and repeated completion).
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let created = common::request(&socket, "POST", "/v1/sessions", "{}");
    let session: Value = serde_json::from_str(created.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let id = session["id"].as_str().unwrap();
    let output = command(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .arg("done")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let path = home
        .path()
        .join("state/sessions")
        .join(id)
        .join("output.md");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    for args in [vec!["done", "-"], vec!["done", "--output", "-"]] {
        let mut child = command(home.path(), &socket)
            .env("WS_SESSION_ID", id)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"alpha \"quoted\" $literal\nbeta")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha \"quoted\" $literal\nbeta"
        );
    }
    let source = home.path().join("result.md");
    std::fs::write(&source, "from file").unwrap();
    let output = command(home.path(), &socket)
        .env("WS_SESSION_ID", id)
        .args(["done", "--output"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "from file");
    for condition in ["done", "event=done", "state=finished"] {
        let output = command(home.path(), &socket)
            .args(["wait", "#1", "--for", condition, "--timeout", "0"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty());
    }
    let output = command(home.path(), &socket)
        .args(["wait", id, "--for", "exit", "--timeout", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let output = command(home.path(), &socket)
        .args(["done", "--output"])
        .arg(home.path().join("missing"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "from file");
    let log = ws_core::log::SessionLog::new(
        &home.path().join("state"),
        id.parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    )
    .read()
    .unwrap();
    assert_eq!(
        log.entries
            .iter()
            .filter(|entry| entry.event_type == "done")
            .count(),
        4
    );
}

#[test]
fn a_supervised_process_records_exit_after_it_declares_done() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let output = command(home.path(), &socket)
        .args([
            "session",
            "create",
            "--headless",
            "--",
            "/bin/sh",
            "-c",
            "\"$1\" done; exit 7",
            "sh",
            env!("CARGO_BIN_EXE_ws"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let response = common::request(
        &socket,
        "POST",
        &format!("/v1/sessions/{id}/wait"),
        r#"{"for":"exit","timeout":0}"#,
    );
    // The wait below handles the race between creation and process exit.
    if response.starts_with("HTTP/1.1 408") {
        let output = command(home.path(), &socket)
            .args(["wait", &id, "--for", "exit", "--timeout", "30"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    } else {
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }
    let fetched = common::get(&socket, &format!("/v1/sessions/{id}"));
    let session: Value = serde_json::from_str(fetched.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(session["exit_code"], 7);
    let entries = ws_core::log::SessionLog::new(
        &home.path().join("state"),
        id.parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    )
    .read()
    .unwrap()
    .entries;
    let done = entries
        .iter()
        .position(|entry| entry.event_type == "done")
        .unwrap();
    let exit = entries
        .iter()
        .position(|entry| entry.event_type == "exit")
        .unwrap();
    assert!(done < exit);
}

#[test]
fn a_session_end_hook_does_not_replace_the_supervisors_exit_status() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let output = command(home.path(), &socket)
        .args([
            "session",
            "create",
            "--headless",
            "--",
            "/bin/sh",
            "-c",
            "printf '{\"exit_code\":0}' | \"$1\" hook exit; exit 7",
            "sh",
            env!("CARGO_BIN_EXE_ws"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let output = command(home.path(), &socket)
        .args(["wait", &id, "--for", "state=failed", "--timeout", "30"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let fetched = common::get(&socket, &format!("/v1/sessions/{id}"));
    let session: Value = serde_json::from_str(fetched.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(session["exit_code"], 7);
}

#[test]
fn daemon_shutdown_releases_an_indefinite_wait() {
    use std::{
        io::Read,
        os::unix::net::UnixStream,
        time::{Duration, Instant},
    };
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let mut daemon = common::Daemon::start(command(home.path(), &socket), &socket);
    let created = common::request(&socket, "POST", "/v1/sessions", "{}");
    let session: Value = serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = session["id"].as_str().unwrap();
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let body = serde_json::json!({"for":"done", "parent_session_id":id}).to_string();
    write!(stream, "POST /v1/sessions/{id}/wait HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let entries = ws_core::log::SessionLog::new(
            &home.path().join("state"),
            id.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries;
        if entries
            .iter()
            .any(|entry| entry.body["operation"] == "session.wait")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "The daemon did not register the wait."
        );
        std::thread::yield_now();
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &daemon.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert!(daemon.child.wait().unwrap().success());
    assert!(!socket.exists());
}
