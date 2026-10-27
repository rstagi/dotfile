use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair,
    PKCS_ED25519, SanType,
};
use ws_hub::identity;

mod common;
use common::{Daemon, ws};

struct Hub {
    child: Child,
    addr: String,
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cli(home: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = ws(home, &home.join("wsd.sock"));
    command.args(arguments).output().unwrap()
}

fn config_hub(home: &Path, addr: &str, device: &str) {
    fs::create_dir_all(home.join("state")).unwrap();
    fs::write(
        home.join("state/config.toml"),
        format!(
            "[hub]\nurl = \"https://{addr}\"\ncert = \"{0}/{device}.pem\"\nkey = \"{0}/{device}.key\"\nca = \"{0}/root.pem\"\n",
            home.display()
        ),
    )
    .unwrap();
}

/// Writes root.pem, server.pem/key and `<id>.pem/key` for each device id.
fn pki(dir: &Path, ids: &[&str]) {
    let root = identity::create_root_ca().unwrap();
    fs::write(dir.join("root.pem"), &root.cert_pem).unwrap();
    fs::write(dir.join("root.key"), &root.key_pem).unwrap();
    let issuer =
        Issuer::from_ca_cert_pem(&root.cert_pem, KeyPair::from_pem(&root.key_pem).unwrap())
            .unwrap();
    let key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![SanType::IpAddress("127.0.0.1".parse().unwrap())];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let cert = params.signed_by(&key, &issuer).unwrap();
    fs::write(dir.join("server.pem"), cert.pem()).unwrap();
    fs::write(dir.join("server.key"), key.serialize_pem()).unwrap();
    for id in ids {
        let key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, *id);
        params.distinguished_name = dn;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = params.signed_by(&key, &issuer).unwrap();
        fs::write(dir.join(format!("{id}.pem")), cert.pem()).unwrap();
        fs::write(dir.join(format!("{id}.key")), key.serialize_pem()).unwrap();
    }
}

fn serve(dir: &Path) -> Hub {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ws"))
        .args(["hub", "serve", "--listen", "127.0.0.1:0"])
        .arg("--tls-cert")
        .arg(dir.join("server.pem"))
        .arg("--tls-key")
        .arg(dir.join("server.key"))
        .arg("--ca-cert")
        .arg(dir.join("root.pem"))
        .arg("--state")
        .arg(dir.join("hub"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // The ready line comes before the first request.
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let addr = line
        .trim()
        .strip_prefix("hub ready ")
        .expect(&line)
        .to_owned();
    Hub { child, addr }
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

fn add_device(dir: &Path, id: &str) {
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

struct Join {
    child: Child,
    home: std::path::PathBuf,
    code: String,
}

impl Drop for Join {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `ws device join` in its own state directory and reads the code it prints.
fn join(dir: &Path, name: &str, addr: &str) -> Join {
    let home = dir.join(format!("join-{name}"));
    fs::create_dir_all(&home).unwrap();
    let mut command = ws(&home, &home.join("wsd.sock"));
    let mut child = command
        .args([
            "device",
            "join",
            &format!("https://{addr}"),
            "--name",
            name,
            "--ca",
        ])
        .arg(dir.join("root.pem"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let code = loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "no pairing code");
        if let Some(code) = line.trim().strip_prefix("Pairing code: ") {
            break code.to_owned();
        }
    };
    // Keep the pipe open so the child does not get SIGPIPE on its next line.
    std::mem::forget(stdout);
    Join { child, home, code }
}

fn approve(dir: &Path, code: &str) -> std::process::Output {
    let key_cmd = format!("cat '{}'", dir.join("root.key").display());
    cli(
        dir,
        &["device", "approve", code, "--root-key-cmd", &key_cmd],
    )
}

fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    for _ in 0..200 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the join command did not end");
}

#[test]
fn join_and_approve_pair_a_new_device() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    config_hub(dir, &hub.addr, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    assert_eq!(joining.code.len(), 8);
    let approved = approve(dir, &joining.code.to_lowercase());
    assert!(approved.status.success(), "{approved:?}");
    assert!(wait_exit(&mut joining.child).success());
    let state = joining.home.join("state");
    assert!(state.join("device/cert.pem").exists());
    let config = fs::read_to_string(state.join("config.toml")).unwrap();
    assert!(
        config.contains("[hub]") && config.contains(&hub.addr),
        "{config}"
    );
    // The new device calls the hub with its own certificate.
    let mut listed = ws(&joining.home, &joining.home.join("wsd.sock"));
    let listed = listed.args(["device", "list", "--json"]).output().unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let mut names: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["dev-a", "mini"]);
}

#[test]
fn used_and_unknown_codes_give_404() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    config_hub(dir, &hub.addr, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    assert!(approve(dir, &joining.code).status.success());
    assert!(wait_exit(&mut joining.child).success());
    for code in [joining.code.as_str(), "ZZZZZZZZ"] {
        let again = approve(dir, code);
        assert!(!again.status.success());
        assert!(
            String::from_utf8_lossy(&again.stderr).contains("404"),
            "{again:?}"
        );
    }
}

#[test]
fn approve_from_an_unregistered_device_gives_403_and_keeps_the_code() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    config_hub(dir, &hub.addr, "dev-b");
    let denied = approve(dir, &joining.code);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("403"),
        "{denied:?}"
    );
    assert!(joining.child.try_wait().unwrap().is_none());
    config_hub(dir, &hub.addr, "dev-a");
    assert!(approve(dir, &joining.code).status.success());
    assert!(wait_exit(&mut joining.child).success());
}
