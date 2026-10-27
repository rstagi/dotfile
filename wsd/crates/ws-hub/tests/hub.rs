use std::{fs, net::SocketAddr, path::Path};

use hyper::Method;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ED25519, SanType,
};
use time::{Duration, OffsetDateTime};
use ws_hub::{
    client::HubClient,
    identity::{self, RootCa},
    registry::Registry,
    server::{self, HubOptions},
};

struct Fixture {
    dir: tempfile::TempDir,
    root: RootCa,
    addr: SocketAddr,
}

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn name(common: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common);
    dn
}

/// Writes `<id>.pem` and `<id>.key`. `days` below zero gives an expired certificate.
fn device(f: &Fixture, id: &str, days: i64) {
    let key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut params = CertificateParams::default();
    params.distinguished_name = name(id);
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(30);
    params.not_after = now + Duration::days(days);
    let issuer = Issuer::from_ca_cert_pem(
        &f.root.cert_pem,
        KeyPair::from_pem(&f.root.key_pem).unwrap(),
    )
    .unwrap();
    let cert = params.signed_by(&key, &issuer).unwrap();
    write(f.dir.path(), &format!("{id}.pem"), &cert.pem());
    write(f.dir.path(), &format!("{id}.key"), &key.serialize_pem());
}

fn client(f: &Fixture, id: &str) -> HubClient {
    HubClient::new(
        &format!("https://{}", f.addr),
        &f.dir.path().join(format!("{id}.pem")),
        &f.dir.path().join(format!("{id}.key")),
        &f.dir.path().join("root.pem"),
    )
    .unwrap()
}

fn add(f: &Fixture, id: &str) {
    Registry::open(&f.dir.path().join("state"))
        .unwrap()
        .add_device(&f.dir.path().join(format!("{id}.pem")), id)
        .unwrap();
}

async fn start() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = identity::create_root_ca().unwrap();
    write(dir.path(), "root.pem", &root.cert_pem);
    let server_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut params = CertificateParams::default();
    params.distinguished_name = name("hub");
    params.subject_alt_names = vec![SanType::IpAddress("127.0.0.1".parse().unwrap())];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let issuer =
        Issuer::from_ca_cert_pem(&root.cert_pem, KeyPair::from_pem(&root.key_pem).unwrap())
            .unwrap();
    let cert = params.signed_by(&server_key, &issuer).unwrap();
    write(dir.path(), "server.pem", &cert.pem());
    write(dir.path(), "server.key", &server_key.serialize_pem());
    let state = dir.path().join("state");
    let hub = server::bind(&HubOptions {
        listen: "127.0.0.1:0".parse().unwrap(),
        tls_cert: dir.path().join("server.pem"),
        tls_key: dir.path().join("server.key"),
        ca_cert: dir.path().join("root.pem"),
        state,
    })
    .await
    .unwrap();
    let addr = hub.local_addr().unwrap();
    tokio::spawn(hub.serve());
    Fixture { dir, root, addr }
}

#[tokio::test]
async fn added_device_lists_devices() {
    let f = start().await;
    device(&f, "dev-a", 30);
    add(&f, "dev-a");
    let response = client(&f, "dev-a")
        .request(Method::GET, "/v1/devices")
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(
        response.body.contains("\"id\":\"dev-a\""),
        "{}",
        response.body
    );
}

#[tokio::test]
async fn device_missing_from_registry_gets_403() {
    let f = start().await;
    device(&f, "dev-a", 30);
    device(&f, "dev-b", 30);
    add(&f, "dev-a");
    let response = client(&f, "dev-b")
        .request(Method::GET, "/v1/devices")
        .await
        .unwrap();
    assert_eq!(response.status, 403);
}

#[tokio::test]
async fn revoked_device_fails_on_next_request_of_the_same_connection() {
    let f = start().await;
    device(&f, "dev-a", 30);
    device(&f, "dev-b", 30);
    add(&f, "dev-a");
    add(&f, "dev-b");
    let mut kept = client(&f, "dev-b").connect().await.unwrap();
    assert_eq!(
        kept.request(Method::GET, "/v1/devices")
            .await
            .unwrap()
            .status,
        200
    );
    let removed = client(&f, "dev-a")
        .request(Method::DELETE, "/v1/devices/dev-b")
        .await
        .unwrap();
    assert_eq!(removed.status, 204);
    assert_eq!(
        kept.request(Method::GET, "/v1/devices")
            .await
            .unwrap()
            .status,
        403
    );
}

#[tokio::test]
async fn expired_certificate_cannot_call_the_hub() {
    let f = start().await;
    device(&f, "dev-old", -1);
    add(&f, "dev-old");
    let result = client(&f, "dev-old")
        .request(Method::GET, "/v1/devices")
        .await;
    assert!(result.is_err(), "{:?}", result.map(|r| r.status));
}

#[tokio::test]
async fn pair_and_mcp_routes_need_no_client_certificate() {
    let f = start().await;
    device(&f, "dev-a", 30);
    // A client without a registered certificate still reaches the public routes.
    for path in ["/v1/pair/x", "/mcp"] {
        let response = client(&f, "dev-a")
            .request(Method::GET, path)
            .await
            .unwrap();
        assert_ne!(response.status, 403, "{path}");
    }
}
