//! Device identity: the WS root CA, the device key and device certificates.
//!
//! The root key never leaves memory in `sign_device_csr`. `read_root_key` takes it
//! from the stdout of a command, so no file, argument, log or environment holds it.

use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, PKCS_ED25519, SanType,
};
use rustls::{
    RootCertStore,
    pki_types::{CertificateDer, UnixTime, pem::PemObject},
    server::WebPkiClientVerifier,
};
use time::{Duration, OffsetDateTime};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub const ROOT_VALIDITY_DAYS: i64 = 3650;
pub const DEFAULT_DEVICE_DAYS: i64 = 90;
pub const ROOT_CERT_FILE: &str = "root-ca.pem";
pub const ROOT_KEY_FILE: &str = "root-ca.key";
pub const DEVICE_CSR_FILE: &str = "csr.pem";
pub const DEFAULT_ROOT_KEY_COMMAND: &str = "op read op://ws/root-ca/key";

const ROOT_COMMON_NAME: &str = "WS Root CA";

pub struct RootCa {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Writes a new root key (mode 0600) and its self-signed certificate into `out`.
/// Returns the paths of the certificate and the key.
pub fn write_root_ca(out: &Path) -> Result<(PathBuf, PathBuf)> {
    let root = create_root_ca()?;
    create_private_dir(out)?;
    let cert_path = out.join(ROOT_CERT_FILE);
    let key_path = out.join(ROOT_KEY_FILE);
    write_new(&key_path, root.key_pem.as_bytes(), 0o600)?;
    write_new(&cert_path, root.cert_pem.as_bytes(), 0o644)?;
    Ok((cert_path, key_path))
}

pub fn create_root_ca() -> Result<RootCa> {
    let key = KeyPair::generate_for(&PKCS_ED25519)?;
    let mut params = CertificateParams::default();
    params.distinguished_name = common_name(ROOT_COMMON_NAME);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    set_validity(&mut params, ROOT_VALIDITY_DAYS);
    let cert = params.self_signed(&key)?;
    Ok(RootCa {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Writes the device key (mode 0600) and a certificate request for `device_id`
/// into `directory`. An existing key stays. Returns true when it wrote a new key.
pub fn init_device_key(directory: &Path, device_id: &str) -> Result<bool> {
    create_private_dir(directory)?;
    let key_path = directory.join("key.pem");
    let (key, created) = match fs::read_to_string(&key_path) {
        Ok(pem) => (KeyPair::from_pem(&pem)?, false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let key = KeyPair::generate_for(&PKCS_ED25519)?;
            write_new(&key_path, key.serialize_pem().as_bytes(), 0o600)?;
            (key, true)
        }
        Err(error) => return Err(error.into()),
    };
    let csr_path = directory.join(DEVICE_CSR_FILE);
    if !csr_path.exists() {
        let mut params = CertificateParams::default();
        params.distinguished_name = common_name(device_id);
        let csr = params.serialize_request(&key)?;
        write_new(&csr_path, csr.pem()?.as_bytes(), 0o644)?;
    }
    Ok(created)
}

/// Signs the key of a certificate request. The certificate subject is the common
/// name of the request. The request cannot add a CA flag or other extensions.
/// The result must chain to the root, which also proves that the key matches.
pub fn sign_device_csr(
    csr_pem: &str,
    root_cert_pem: &str,
    root_key_pem: &str,
    days: i64,
) -> Result<String> {
    if days < 1 {
        return Err("The validity must be at least 1 day.".into());
    }
    let mut request = CertificateSigningRequestParams::from_pem(csr_pem)?;
    let device_id = request
        .params
        .distinguished_name
        .get(&DnType::CommonName)
        .and_then(|value| match value {
            rcgen::DnValue::Utf8String(text) => Some(text.clone()),
            rcgen::DnValue::PrintableString(text) => Some(text.to_string()),
            _ => None,
        })
        .filter(|id| !id.is_empty())
        .ok_or("The request has no device id as common name.")?;
    let mut params = CertificateParams::default();
    params.distinguished_name = common_name(&device_id);
    params.subject_alt_names = vec![SanType::DnsName(device_id.as_str().try_into()?)];
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ClientAuth,
        ExtendedKeyUsagePurpose::ServerAuth,
    ];
    set_validity(&mut params, days);
    request.params = params;
    let issuer = Issuer::from_ca_cert_pem(root_cert_pem, KeyPair::from_pem(root_key_pem)?)?;
    let cert_pem = request.signed_by(&issuer)?.pem();
    verify_device_cert(root_cert_pem, &cert_pem)?;
    Ok(cert_pem)
}

/// Runs `command` through `sh -c` and returns its stdout as the root key.
pub fn read_root_key(command: &str) -> Result<String> {
    let output = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(format!("The root key command failed: {}.", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Checks the chain with rustls and returns the subject common name (the device id).
pub fn verify_device_cert(root_cert_pem: &str, device_cert_pem: &str) -> Result<String> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(root_cert_pem.as_bytes())?)?;
    let verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )
    .build()?;
    let device = CertificateDer::from_pem_slice(device_cert_pem.as_bytes())?;
    verifier.verify_client_cert(&device, &[], UnixTime::now())?;
    certificate_subject(device_cert_pem)
}

pub fn certificate_subject(cert_pem: &str) -> Result<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())?;
    let cert = pem.parse_x509()?;
    let name = cert
        .subject()
        .iter_common_name()
        .next()
        .ok_or("The certificate has no common name.")?
        .as_str()?;
    Ok(name.to_owned())
}

fn common_name(name: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, name);
    dn
}

fn set_validity(params: &mut CertificateParams, days: i64) {
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::minutes(5);
    params.not_after = now + Duration::days(days);
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn write_new(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const DEVICE_ID: &str = "01JABCDEFGHJKMNPQRSTVWXYZ0";

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn signed_device(root: &RootCa, id: &str) -> String {
        let sandbox = tempfile::tempdir().unwrap();
        init_device_key(sandbox.path(), id).unwrap();
        let csr = fs::read_to_string(sandbox.path().join(DEVICE_CSR_FILE)).unwrap();
        sign_device_csr(&csr, &root.cert_pem, &root.key_pem, DEFAULT_DEVICE_DAYS).unwrap()
    }

    #[test]
    fn root_ca_files_hold_a_private_key_and_a_ten_year_certificate() {
        let sandbox = tempfile::tempdir().unwrap();
        let out = sandbox.path().join("ca");
        let (cert, key) = write_root_ca(&out).unwrap();
        assert_eq!(mode(&key), 0o600);
        assert_eq!(mode(&out), 0o700);
        let pem = fs::read_to_string(&cert).unwrap();
        let (_, parsed) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap();
        let cert = parsed.parse_x509().unwrap();
        let days = (cert.validity().not_after.timestamp() - cert.validity().not_before.timestamp())
            / 86_400;
        assert!((ROOT_VALIDITY_DAYS..=ROOT_VALIDITY_DAYS + 1).contains(&days));
        assert!(cert.is_ca());
        assert!(write_root_ca(&out).is_err(), "must not overwrite a root key");
    }

    #[test]
    fn device_init_is_idempotent_and_keeps_private_modes() {
        let sandbox = tempfile::tempdir().unwrap();
        let dir = sandbox.path().join("device");
        assert!(init_device_key(&dir, DEVICE_ID).unwrap());
        let key = fs::read(dir.join("key.pem")).unwrap();
        let csr = fs::read(dir.join(DEVICE_CSR_FILE)).unwrap();
        assert_eq!(mode(&dir.join("key.pem")), 0o600);
        assert_eq!(mode(&dir), 0o700);
        assert!(!init_device_key(&dir, DEVICE_ID).unwrap());
        assert_eq!(fs::read(dir.join("key.pem")).unwrap(), key);
        assert_eq!(fs::read(dir.join(DEVICE_CSR_FILE)).unwrap(), csr);
    }

    #[test]
    fn device_init_rebuilds_a_missing_request_from_the_existing_key() {
        let sandbox = tempfile::tempdir().unwrap();
        init_device_key(sandbox.path(), DEVICE_ID).unwrap();
        let key = fs::read(sandbox.path().join("key.pem")).unwrap();
        fs::remove_file(sandbox.path().join(DEVICE_CSR_FILE)).unwrap();
        assert!(!init_device_key(sandbox.path(), DEVICE_ID).unwrap());
        assert_eq!(fs::read(sandbox.path().join("key.pem")).unwrap(), key);
        assert!(sandbox.path().join(DEVICE_CSR_FILE).exists());
    }

    #[test]
    fn signed_certificate_has_the_device_id_and_chains_to_the_root() {
        let root = create_root_ca().unwrap();
        let cert = signed_device(&root, DEVICE_ID);
        assert_eq!(verify_device_cert(&root.cert_pem, &cert).unwrap(), DEVICE_ID);
        assert_eq!(certificate_subject(&cert).unwrap(), DEVICE_ID);
    }

    #[test]
    fn certificate_from_another_root_fails_the_check() {
        let root = create_root_ca().unwrap();
        let other = create_root_ca().unwrap();
        let cert = signed_device(&other, DEVICE_ID);
        assert!(verify_device_cert(&root.cert_pem, &cert).is_err());
    }

    #[test]
    fn signing_with_a_key_that_does_not_match_the_root_certificate_fails() {
        let root = create_root_ca().unwrap();
        let other = create_root_ca().unwrap();
        let sandbox = tempfile::tempdir().unwrap();
        init_device_key(sandbox.path(), DEVICE_ID).unwrap();
        let csr = fs::read_to_string(sandbox.path().join(DEVICE_CSR_FILE)).unwrap();
        assert!(sign_device_csr(&csr, &root.cert_pem, &other.key_pem, 90).is_err());
    }

    #[test]
    fn root_key_comes_from_the_command_stdout() {
        let sandbox = tempfile::tempdir().unwrap();
        let (_, key_path) = write_root_ca(sandbox.path()).unwrap();
        let key = read_root_key(&format!("cat '{}'", key_path.display())).unwrap();
        assert_eq!(key, fs::read_to_string(&key_path).unwrap());
        assert!(read_root_key("exit 3").is_err());
    }
}
