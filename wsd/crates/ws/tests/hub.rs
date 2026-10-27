use std::{fs, path::Path};

mod common;
use common::{
    Daemon,
    hub_fixture::{config_hub, pki, serve},
    ws,
};

fn cli(home: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = ws(home, &home.join("wsd.sock"));
    command.args(arguments).output().unwrap()
}

#[test]
fn hub_commands_without_hub_section_print_one_error_and_exit_2() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    for arguments in [vec!["device", "list"], vec!["device", "remove", "x"]] {
        let output = cli(sandbox.path(), &arguments);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr).lines().count(), 1);
    }
}

#[test]
fn wsd_starts_with_and_without_hub_section() {
    for with_hub in [false, true] {
        let sandbox = tempfile::tempdir_in("/tmp").unwrap();
        let socket = sandbox.path().join("wsd.sock");
        if with_hub {
            config_hub(sandbox.path(), "127.0.0.1:1", "dev-a");
        }
        let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    }
}

#[test]
fn device_commands_list_and_revoke_through_the_hub() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    for id in ["dev-a", "dev-b"] {
        let output = cli(
            dir,
            &[
                "hub",
                "add-device",
                "--state",
                dir.join("hub").to_str().unwrap(),
                "--cert",
                dir.join(format!("{id}.pem")).to_str().unwrap(),
                "--name",
                id,
            ],
        );
        assert!(output.status.success(), "{output:?}");
    }
    config_hub(dir, &hub.addr, "dev-a");
    let listed = cli(dir, &["device", "list", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    let removed = cli(dir, &["device", "remove", "dev-b"]);
    assert!(removed.status.success(), "{removed:?}");
    config_hub(dir, &hub.addr, "dev-b");
    let denied = cli(dir, &["device", "list"]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("403"),
        "{denied:?}"
    );
}
