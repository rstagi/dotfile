use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
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
