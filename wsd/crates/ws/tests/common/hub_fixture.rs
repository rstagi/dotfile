//! A test hub: certificates, a `ws hub serve` child and the `[hub]` config of a device.

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

pub struct Hub {
    pub child: Child,
    pub addr: String,
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn config_hub(home: &Path, addr: &str, device: &str) {
    config_hub_with(home, home, addr, device);
}

/// Writes the `[hub]` section of `home` with the certificates of `pki_dir`.
pub fn config_hub_with(home: &Path, pki_dir: &Path, addr: &str, device: &str) {
    fs::create_dir_all(home.join("state")).unwrap();
    fs::write(
        home.join("state/config.toml"),
        format!(
            "[hub]\nurl = \"https://{addr}\"\ncert = \"{0}/{device}.pem\"\nkey = \"{0}/{device}.key\"\nca = \"{0}/root.pem\"\n",
            pki_dir.display()
        ),
    )
    .unwrap();
}

/// Writes root.pem, server.pem/key and `<id>.pem/key` for each device id.
pub fn pki(dir: &Path, ids: &[&str]) {
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

pub fn serve(dir: &Path) -> Hub {
    serve_on(dir, "127.0.0.1:0")
}

/// Starts the hub on `listen`. A restart on the address of a stopped hub keeps its state.
pub fn serve_on(dir: &Path, listen: &str) -> Hub {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ws"))
        .args(["hub", "serve", "--listen", listen])
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
