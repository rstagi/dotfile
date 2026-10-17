use std::{
    os::unix::fs::PermissionsExt,
    process::Command,
    sync::Barrier,
    thread,
    time::{Duration, Instant},
};

mod common;
use common::{Daemon, create_stale_socket, get, request, ws};

#[test]
fn daemon_serves_versioned_json_and_ping_shows_version_and_uptime() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);

    let response = get(&socket, "/v1/ping");
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(headers.contains("application/json"), "{response}");
    let ping: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(ping["version"], env!("CARGO_PKG_VERSION"));
    assert!(ping["uptime_secs"].as_u64().is_some(), "{ping}");

    thread::sleep(Duration::from_millis(1100));
    let output = ws(sandbox.path(), &socket).arg("ping").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    let uptime: u64 = text
        .trim()
        .strip_prefix(&format!("wsd {} (uptime: ", env!("CARGO_PKG_VERSION")))
        .unwrap()
        .strip_suffix("s)")
        .unwrap()
        .parse()
        .unwrap();
    assert!(uptime >= 1, "{text}");

    assert!(get(&socket, "/ping").starts_with("HTTP/1.1 404"));
}

#[test]
fn ping_without_daemon_reports_the_socket_and_exits_one() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("missing.sock");
    let output = ws(sandbox.path(), &socket).arg("ping").output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Cannot connect to the daemon"), "{error}");
    assert!(error.contains(socket.to_str().unwrap()), "{error}");
}

#[test]
fn daemon_creates_the_default_socket_and_removes_it_on_shutdown() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join(".ws/wsd.sock");
    let mut command = ws(sandbox.path(), &socket);
    command.env_remove("WS_SOCK");
    let mut daemon = Daemon::start(command, &socket);

    let output = ws(sandbox.path(), &socket)
        .env_remove("WS_SOCK")
        .arg("ping")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        socket.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );

    assert!(
        Command::new("kill")
            .args(["-TERM", &daemon.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = daemon.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "The daemon did not stop.");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "{status}");
    assert!(!socket.exists());

    let _restarted = Daemon::start(ws(sandbox.path(), &socket), &socket);
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn another_daemon_cannot_replace_a_live_socket_or_an_existing_file() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let output = ws(sandbox.path(), &socket).arg("daemon").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200 OK"));

    let file = sandbox.path().join("keep.txt");
    std::fs::write(&file, "Keep this file.").unwrap();
    let output = ws(sandbox.path(), &file).arg("daemon").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "Keep this file.");
}

#[test]
fn daemon_refuses_a_held_socket_lock_before_binding() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let lock = std::fs::File::create(socket.with_extension("sock.lock")).unwrap();
    lock.try_lock().unwrap();

    let mut daemon = Daemon::spawn(ws(sandbox.path(), &socket));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = daemon.child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(1));
            let error = daemon.stderr();
            assert!(
                error.contains("Another daemon holds the socket lock"),
                "{error}"
            );
            assert!(!socket.exists());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "The daemon ignored the socket lock."
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn daemon_replaces_a_stale_socket() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    create_stale_socket(&socket);
    assert!(socket.exists());

    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn simultaneous_daemons_on_a_stale_socket_leave_exactly_one_server() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    create_stale_socket(&socket);
    let barrier = Barrier::new(2);
    let [mut first, mut second] = thread::scope(|scope| {
        let starts = [0, 1].map(|_| {
            scope.spawn(|| {
                barrier.wait();
                Daemon::spawn(ws(sandbox.path(), &socket))
            })
        });
        starts.map(|start| start.join().unwrap())
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let (winner, loser, status) = loop {
        match (
            first.child.try_wait().unwrap(),
            second.child.try_wait().unwrap(),
        ) {
            (None, Some(status)) => break (&mut first, &second, status),
            (Some(status), None) => break (&mut second, &first, status),
            (Some(_), Some(_)) => panic!("Both daemons exited."),
            (None, None) => {}
        }
        assert!(Instant::now() < deadline, "Both daemons remained alive.");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1), "{}", loser.stderr());
    assert!(
        loser
            .stderr()
            .contains("Another daemon holds the socket lock"),
        "{}",
        loser.stderr()
    );
    winner.wait_ready(&socket);
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200 OK"));
    assert!(winner.child.try_wait().unwrap().is_none());
}

#[test]
fn daemon_releases_its_lock_after_an_abrupt_exit() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let mut daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let lock = std::fs::File::open(socket.with_extension("sock.lock")).unwrap();
    assert!(matches!(
        lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));

    daemon.child.kill().unwrap();
    daemon.child.wait().unwrap();
    assert!(socket.exists());
    let _restarted = Daemon::start(ws(sandbox.path(), &socket), &socket);
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn ping_to_a_stale_socket_exits_one() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("stale.sock");
    create_stale_socket(&socket);
    let output = ws(sandbox.path(), &socket).arg("ping").output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Cannot connect to the daemon"), "{error}");
    assert!(error.contains(socket.to_str().unwrap()), "{error}");
}

#[test]
fn hook_without_a_tmux_pane_reports_the_required_context() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("missing.sock");
    let output = ws(sandbox.path(), &socket)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("WS_TMUX_SOCKET")
        .args(["hook", "stop"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("inside a tmux pane")
    );
}

#[test]
fn hook_api_rejects_invalid_records_and_exposes_accepted_events() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    for window in ["@", "@-1", "%1", "spike:1"] {
        let body = serde_json::json!({"event": "stop", "window_id": window}).to_string();
        let response = request(&socket, "POST", "/v1/hooks", &body);
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    }
    let response = request(
        &socket,
        "POST",
        "/v1/hooks",
        r#"{"event":"unknown","window_id":"@7"}"#,
    );
    assert!(response.starts_with("HTTP/1.1 422"), "{response}");
    assert_eq!(
        get(&socket, "/v1/hooks").split_once("\r\n\r\n").unwrap().1,
        "[]"
    );

    let body = r#"{"event":"stop","window_id":"@7"}"#;
    let response = request(&socket, "POST", "/v1/hooks", body);
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let response = get(&socket, "/v1/hooks");
    let records: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        records,
        serde_json::json!([{"event":"stop", "window_id":"@7"}])
    );
}
