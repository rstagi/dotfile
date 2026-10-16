use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Daemon(Child);

#[test]
fn daemon_serves_versioned_json_and_ping_shows_version_and_uptime() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = start_daemon(sandbox.path(), &socket);

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

fn start_daemon(home: &Path, socket: &Path) -> Daemon {
    let mut daemon = Daemon(
        ws(home, socket)
            .arg("daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if UnixStream::connect(socket).is_ok() {
            return daemon;
        }
        if let Some(status) = daemon.0.try_wait().unwrap() {
            let mut error = String::new();
            daemon
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            panic!("The daemon exited with {status}: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "The daemon did not create the socket."
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn ws(home: &Path, socket: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ws"));
    command
        .env("HOME", home)
        .env("WS_HOME", home.join(".ws"))
        .env("WS_STATE", home.join("state"))
        .env("WS_SOCK", socket);
    command
}

fn get(socket: &Path, path: &str) -> String {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
