use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};

use ws_core::device::Device;
use ws_hub::identity;

fn ws(state: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ws"))
        .args(arguments)
        .env_remove("HOME")
        .env_remove("WS_SOCK")
        .env("WS_STATE", state)
        .output()
        .unwrap()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn device_init_keeps_the_device_id_and_is_idempotent() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let state = sandbox.path().join("state");
    let output = ws(&state, &["device", "init"]);
    assert!(output.status.success(), "{output:?}");
    let device = Device::load(&state).unwrap();
    let directory = state.join("device");
    assert_eq!(mode(&directory.join("key.pem")), 0o600);
    let key = fs::read(directory.join("key.pem")).unwrap();
    let csr = fs::read(directory.join("csr.pem")).unwrap();
    let json = fs::read(state.join("device.json")).unwrap();
    assert!(ws(&state, &["device", "init"]).status.success());
    assert_eq!(fs::read(directory.join("key.pem")).unwrap(), key);
    assert_eq!(fs::read(directory.join("csr.pem")).unwrap(), csr);
    assert_eq!(fs::read(state.join("device.json")).unwrap(), json);
    assert_eq!(Device::load(&state).unwrap(), device);
}

#[test]
fn ca_init_then_device_sign_gives_a_certificate_with_the_device_id() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let state = sandbox.path().join("state");
    let ca = sandbox.path().join("ca");
    let output = ws(&state, &["ca", "init", "--out", ca.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("op document create"), "{stdout}");
    assert_eq!(mode(&ca.join("root-ca.key")), 0o600);
    assert!(ws(&state, &["device", "init"]).status.success());
    let device = Device::load(&state).unwrap();
    let key_command = format!("cat '{}'", ca.join("root-ca.key").display());
    let output = ws(
        &state,
        &[
            "device",
            "sign",
            "--csr",
            state.join("device/csr.pem").to_str().unwrap(),
            "--root-cert",
            ca.join("root-ca.pem").to_str().unwrap(),
            "--root-key-cmd",
            &key_command,
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let certificate = String::from_utf8(output.stdout).unwrap();
    let root = fs::read_to_string(ca.join("root-ca.pem")).unwrap();
    assert_eq!(
        identity::verify_device_cert(&root, &certificate).unwrap(),
        device.device_id.to_string()
    );
    assert_eq!(
        identity::certificate_subject(&certificate).unwrap(),
        device.device_id.to_string()
    );
}
