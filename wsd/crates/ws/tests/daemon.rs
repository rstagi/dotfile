use std::process::Command;

#[test]
fn ping_without_daemon_reports_the_socket_and_exits_one() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("missing.sock");
    let output = Command::new(env!("CARGO_BIN_EXE_ws"))
        .arg("ping")
        .env("HOME", sandbox.path())
        .env("WS_SOCK", &socket)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Cannot connect to the daemon"), "{error}");
    assert!(error.contains(socket.to_str().unwrap()), "{error}");
}
