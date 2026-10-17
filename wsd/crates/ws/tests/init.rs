use std::{fs, process::Command};

use ws_core::device::Device;

#[test]
fn init_writes_device_identity_at_ws_state_and_is_idempotent() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let state = sandbox.path().join("custom-state");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_ws"))
            .arg("init")
            .env_remove("HOME")
            .env_remove("WS_SOCK")
            .env("WS_HOME", sandbox.path().join("unused-home"))
            .env("WS_STATE", &state)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{output:?}");
    let device = Device::load(&state).unwrap();
    let hostname = Command::new("hostname").output().unwrap();
    assert!(hostname.status.success());
    assert_eq!(
        device.hostname,
        String::from_utf8(hostname.stdout).unwrap().trim()
    );
    let before = fs::read(state.join("device.json")).unwrap();
    assert!(run().status.success());
    assert_eq!(fs::read(state.join("device.json")).unwrap(), before);
    assert!(!sandbox.path().join("unused-home").exists());
}

#[test]
fn init_uses_the_default_state_under_the_sandbox_home() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ws"))
        .arg("init")
        .env("HOME", sandbox.path())
        .env("WS_HOME", sandbox.path().join("unused-home"))
        .env_remove("WS_STATE")
        .env_remove("WS_SOCK")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(Device::load(&sandbox.path().join(".ws/state")).is_ok());
    assert!(!sandbox.path().join(".ws/wsd.sock").exists());
}
