//! Device pairing. A new device sends a certificate request and gets a short code.
//! A device that the registry has approves the code with a signed certificate.
//!
//! Routes (the first and the third need no client certificate):
//! - `POST /v1/pair` `{name, csr}` → `{code}`
//! - `GET /v1/pair/<code>/cert` waits for the certificate (long poll): 200 `{cert, ca}`,
//!   204 when the wait ends with no certificate, 404 for an unknown, used or expired code.
//! - `GET /v1/pair/<code>/csr` (registered device) → `{name, csr}`
//! - `POST /v1/pair/<code>/cert` `{cert}` (registered device) → 204. The hub checks the
//!   certificate, adds the device to the registry and wakes the waiting `GET`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use http_body_util::{BodyExt, Limited};
use hyper::{Method, Request, Response, StatusCode, body::Bytes, body::Incoming};
use rcgen::CertificateSigningRequestParams;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use x509_parser::prelude::FromDer;

use crate::{
    client::HubClient,
    identity::{Result, verify_device_cert},
    registry::Registry,
    server::{json, reply},
};

pub const CODE_LENGTH: usize = 8;
pub const CODE_TTL: Duration = Duration::from_secs(600);
const POLL_WAIT: Duration = Duration::from_secs(25);
const MAX_BODY: usize = 16 * 1024;
const MAX_PENDING: usize = 64;
const MAX_NAME: usize = 64;
// No 0, 1, I, O: the code is typed by hand.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Serialize, Deserialize)]
pub struct PairStart {
    pub name: String,
    pub csr: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairCode {
    pub code: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairRequest {
    pub name: String,
    pub csr: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairApproval {
    pub cert: String,
}

/// The answer to the joining device: its certificate and the root certificate.
#[derive(Serialize, Deserialize)]
pub struct PairResult {
    pub cert: String,
    pub ca: String,
}

pub struct Pairing {
    registry: Arc<Registry>,
    root_pem: String,
    ttl: Duration,
    poll_wait: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    name: String,
    csr: String,
    device_id: String,
    public_key: Vec<u8>,
    expires: Instant,
    cert: Option<String>,
    ready: Arc<Notify>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    /// The code is unknown, used or expired.
    NotFound,
    Invalid(String),
    Conflict(String),
    Internal(String),
}

enum Take {
    Ready(String),
    Pending(Arc<Notify>),
}

impl Pairing {
    pub fn new(registry: Arc<Registry>, root_pem: String) -> Self {
        Self::with_timing(registry, root_pem, CODE_TTL, POLL_WAIT)
    }

    pub fn with_timing(
        registry: Arc<Registry>,
        root_pem: String,
        ttl: Duration,
        poll_wait: Duration,
    ) -> Self {
        Self {
            registry,
            root_pem,
            ttl,
            poll_wait,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Serves the `/v1/pair` routes. `fingerprint` is the client certificate, if any.
    pub async fn handle(
        &self,
        fingerprint: Option<&str>,
        request: Request<Incoming>,
    ) -> Response<http_body_util::Full<Bytes>> {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        match (method, parts.as_slice()) {
            (Method::POST, ["v1", "pair"]) => match read_json::<PairStart>(request).await {
                Ok(start) => self
                    .open(&start.name, &start.csr, Instant::now())
                    .map_or_else(error_response, |code| {
                        json(StatusCode::OK, &PairCode { code })
                    }),
                Err(response) => response,
            },
            (Method::GET, ["v1", "pair", code, "cert"]) => self.wait(code).await,
            (Method::GET, ["v1", "pair", code, "csr"]) => {
                if let Some(response) = self.deny_unregistered(fingerprint) {
                    return response;
                }
                self.request(code, Instant::now())
                    .map_or_else(error_response, |(name, csr)| {
                        json(StatusCode::OK, &PairRequest { name, csr })
                    })
            }
            (Method::POST, ["v1", "pair", code, "cert"]) => {
                if let Some(response) = self.deny_unregistered(fingerprint) {
                    return response;
                }
                match read_json::<PairApproval>(request).await {
                    Ok(approval) => self
                        .approve(code, &approval.cert, Instant::now())
                        .map_or_else(error_response, |()| reply(StatusCode::NO_CONTENT, "")),
                    Err(response) => response,
                }
            }
            _ => reply(StatusCode::NOT_FOUND, "Unknown route."),
        }
    }

    /// Stores a request and returns its code.
    pub fn open(
        &self,
        name: &str,
        csr: &str,
        now: Instant,
    ) -> std::result::Result<String, PairError> {
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
            return Err(PairError::Invalid("The device name is not valid.".into()));
        }
        let (device_id, public_key) =
            read_csr(csr).map_err(|error| PairError::Invalid(error.to_string()))?;
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| entry.expires > now);
        if entries.len() >= MAX_PENDING {
            return Err(PairError::Conflict(
                "Too many pending pairing codes.".into(),
            ));
        }
        let code = loop {
            let code = random_code().map_err(|error| PairError::Internal(error.to_string()))?;
            if !entries.contains_key(&code) {
                break code;
            }
        };
        entries.insert(
            code.clone(),
            Entry {
                name: name.to_owned(),
                csr: csr.to_owned(),
                device_id,
                public_key,
                expires: now + self.ttl,
                cert: None,
                ready: Arc::new(Notify::new()),
            },
        );
        Ok(code)
    }

    /// Returns the name and the request of a code that no one approved.
    pub fn request(
        &self,
        code: &str,
        now: Instant,
    ) -> std::result::Result<(String, String), PairError> {
        let entries = self.entries.lock().unwrap();
        match entries.get(&normalize(code)) {
            Some(entry) if entry.expires > now && entry.cert.is_none() => {
                Ok((entry.name.clone(), entry.csr.clone()))
            }
            _ => Err(PairError::NotFound),
        }
    }

    /// Checks the certificate, adds the device and wakes the joining device.
    /// A code works one time: a second approval gets `NotFound`.
    pub fn approve(
        &self,
        code: &str,
        cert: &str,
        now: Instant,
    ) -> std::result::Result<(), PairError> {
        let code = normalize(code);
        // The lock stays held, so two approvals of one code cannot both pass.
        let mut entries = self.entries.lock().unwrap();
        let entry = match entries.get_mut(&code) {
            Some(entry) if entry.expires > now && entry.cert.is_none() => entry,
            _ => return Err(PairError::NotFound),
        };
        let subject = verify_device_cert(&self.root_pem, cert).map_err(|error| {
            PairError::Invalid(format!("The certificate is not valid: {error}"))
        })?;
        if subject != entry.device_id
            || certificate_public_key(cert).ok().as_deref() != Some(&entry.public_key)
        {
            return Err(PairError::Invalid(
                "The certificate does not belong to the request.".into(),
            ));
        }
        self.registry
            .add_device_pem(cert, &entry.name)
            .map_err(|error| PairError::Conflict(error.to_string()))?;
        entry.cert = Some(cert.to_owned());
        entry.ready.notify_one();
        Ok(())
    }

    /// Waits for the certificate of a code. The wait ends on the approval event, or
    /// after the poll time with a 204 so that the caller asks again.
    async fn wait(&self, code: &str) -> Response<http_body_util::Full<Bytes>> {
        match self.take(code, Instant::now()) {
            Err(error) => error_response(error),
            Ok(Take::Ready(cert)) => self.result(cert),
            Ok(Take::Pending(ready)) => {
                if tokio::time::timeout(self.poll_wait, ready.notified())
                    .await
                    .is_err()
                {
                    return reply(StatusCode::NO_CONTENT, "");
                }
                match self.take(code, Instant::now()) {
                    Err(error) => error_response(error),
                    Ok(Take::Ready(cert)) => self.result(cert),
                    Ok(Take::Pending(_)) => reply(StatusCode::NO_CONTENT, ""),
                }
            }
        }
    }

    /// Hands out the certificate one time and then forgets the code.
    fn take(&self, code: &str, now: Instant) -> std::result::Result<Take, PairError> {
        let code = normalize(code);
        let mut entries = self.entries.lock().unwrap();
        let entry = match entries.get(&code) {
            Some(entry) if entry.expires > now => entry,
            _ => {
                entries.remove(&code);
                return Err(PairError::NotFound);
            }
        };
        match &entry.cert {
            Some(cert) => {
                let cert = cert.clone();
                entries.remove(&code);
                Ok(Take::Ready(cert))
            }
            None => Ok(Take::Pending(entry.ready.clone())),
        }
    }

    fn result(&self, cert: String) -> Response<http_body_util::Full<Bytes>> {
        json(
            StatusCode::OK,
            &PairResult {
                cert,
                ca: self.root_pem.clone(),
            },
        )
    }

    fn deny_unregistered(
        &self,
        fingerprint: Option<&str>,
    ) -> Option<Response<http_body_util::Full<Bytes>>> {
        match fingerprint.map(|print| self.registry.active_by_fingerprint(print)) {
            Some(Ok(Some(_))) => None,
            Some(Err(error)) => Some(reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())),
            _ => Some(reply(StatusCode::FORBIDDEN, "Device not allowed.")),
        }
    }
}

/// Sends a request to the hub. Returns the code to show to the user.
pub async fn start(client: &HubClient, name: &str, csr: &str) -> Result<String> {
    let body = serde_json::to_string(&PairStart {
        name: name.to_owned(),
        csr: csr.to_owned(),
    })?;
    let response = client.request_json(Method::POST, "/v1/pair", &body).await?;
    expect(&response, 200)?;
    Ok(serde_json::from_str::<PairCode>(&response.body)?.code)
}

/// Waits for the certificate. The hub answers when the approval happens.
pub async fn wait_for_certificate(client: &HubClient, code: &str) -> Result<PairResult> {
    loop {
        let response = client
            .request(Method::GET, &format!("/v1/pair/{code}/cert"))
            .await?;
        match response.status {
            200 => return Ok(serde_json::from_str(&response.body)?),
            204 => {}
            _ => return Err(unexpected(&response)),
        }
    }
}

/// Reads the name and the request of a code. Run it with a registered device.
pub async fn fetch_request(client: &HubClient, code: &str) -> Result<PairRequest> {
    let response = client
        .request(Method::GET, &format!("/v1/pair/{code}/csr"))
        .await?;
    expect(&response, 200)?;
    Ok(serde_json::from_str(&response.body)?)
}

/// Sends the signed certificate. The hub adds the device to the registry.
pub async fn send_certificate(client: &HubClient, code: &str, cert: &str) -> Result<()> {
    let body = serde_json::to_string(&PairApproval {
        cert: cert.to_owned(),
    })?;
    let response = client
        .request_json(Method::POST, &format!("/v1/pair/{code}/cert"), &body)
        .await?;
    expect(&response, 204)
}

fn expect(response: &crate::client::HubResponse, status: u16) -> Result<()> {
    if response.status == status {
        Ok(())
    } else {
        Err(unexpected(response))
    }
}

fn unexpected(response: &crate::client::HubResponse) -> Box<dyn std::error::Error + Send + Sync> {
    format!("The hub answered {}: {}", response.status, response.body).into()
}

fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn random_code() -> Result<String> {
    let mut bytes = [0u8; CODE_LENGTH];
    rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| "Cannot read random bytes.")?;
    // 256 is a multiple of the alphabet size, so the modulo adds no bias.
    Ok(bytes
        .iter()
        .map(|byte| ALPHABET[*byte as usize % ALPHABET.len()] as char)
        .collect())
}

/// Checks the request signature. Returns the device id and the public key bytes.
fn read_csr(csr: &str) -> Result<(String, Vec<u8>)> {
    CertificateSigningRequestParams::from_pem(csr)?;
    let (_, pem) = x509_parser::pem::parse_x509_pem(csr.as_bytes())?;
    let (_, parsed) =
        x509_parser::certification_request::X509CertificationRequest::from_der(&pem.contents)?;
    let info = &parsed.certification_request_info;
    let id = info
        .subject
        .iter_common_name()
        .next()
        .ok_or("The request has no device id as common name.")?
        .as_str()?
        .to_owned();
    Ok((id, info.subject_pki.subject_public_key.data.to_vec()))
}

fn certificate_public_key(cert_pem: &str) -> Result<Vec<u8>> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())?;
    let cert = pem.parse_x509()?;
    Ok(cert.public_key().subject_public_key.data.to_vec())
}

async fn read_json<T: serde::de::DeserializeOwned>(
    request: Request<Incoming>,
) -> std::result::Result<T, Response<http_body_util::Full<Bytes>>> {
    let body = Limited::new(request.into_body(), MAX_BODY)
        .collect()
        .await
        .map_err(|_| reply(StatusCode::PAYLOAD_TOO_LARGE, "The body is too large."))?
        .to_bytes();
    serde_json::from_slice(&body)
        .map_err(|error| reply(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {error}")))
}

fn error_response(error: PairError) -> Response<http_body_util::Full<Bytes>> {
    match error {
        PairError::NotFound => reply(StatusCode::NOT_FOUND, "Unknown, used or expired code."),
        PairError::Invalid(message) => reply(StatusCode::BAD_REQUEST, &message),
        PairError::Conflict(message) => reply(StatusCode::CONFLICT, &message),
        PairError::Internal(message) => reply(StatusCode::INTERNAL_SERVER_ERROR, &message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{RootCa, create_root_ca, init_device_key, sign_device_csr};

    const ID: &str = "01JABCDEFGHJKMNPQRSTVWXYZ0";

    struct Fixture {
        _dir: tempfile::TempDir,
        root: RootCa,
        pairing: Pairing,
        csr: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = create_root_ca().unwrap();
        let registry = Arc::new(Registry::open(&dir.path().join("hub")).unwrap());
        init_device_key(&dir.path().join("device"), ID).unwrap();
        let csr = std::fs::read_to_string(dir.path().join("device/csr.pem")).unwrap();
        let pairing = Pairing::new(registry, root.cert_pem.clone());
        Fixture {
            _dir: dir,
            root,
            pairing,
            csr,
        }
    }

    fn sign(f: &Fixture) -> String {
        sign_device_csr(&f.csr, &f.root.cert_pem, &f.root.key_pem, 30).unwrap()
    }

    #[test]
    fn code_has_eight_characters_from_the_alphabet() {
        let f = fixture();
        let code = f.pairing.open("laptop", &f.csr, Instant::now()).unwrap();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|byte| ALPHABET.contains(&byte)));
    }

    #[test]
    fn approve_adds_the_device_and_the_code_works_one_time() {
        let f = fixture();
        let now = Instant::now();
        let code = f.pairing.open("laptop", &f.csr, now).unwrap();
        assert_eq!(f.pairing.request(&code, now).unwrap().0, "laptop");
        f.pairing
            .approve(&code.to_lowercase(), &sign(&f), now)
            .unwrap();
        assert_eq!(f.pairing.registry.list().unwrap()[0].id, ID);
        assert_eq!(
            f.pairing.approve(&code, &sign(&f), now),
            Err(PairError::NotFound)
        );
        assert_eq!(f.pairing.request(&code, now), Err(PairError::NotFound));
        assert!(matches!(f.pairing.take(&code, now), Ok(Take::Ready(_))));
        assert!(matches!(
            f.pairing.take(&code, now),
            Err(PairError::NotFound)
        ));
    }

    #[test]
    fn code_expires_after_ten_minutes() {
        let f = fixture();
        let now = Instant::now();
        let code = f.pairing.open("laptop", &f.csr, now).unwrap();
        let almost = now + CODE_TTL - Duration::from_secs(1);
        assert!(f.pairing.request(&code, almost).is_ok());
        let late = now + CODE_TTL;
        assert_eq!(f.pairing.request(&code, late), Err(PairError::NotFound));
        assert_eq!(
            f.pairing.approve(&code, &sign(&f), late),
            Err(PairError::NotFound)
        );
        assert!(matches!(
            f.pairing.take(&code, late),
            Err(PairError::NotFound)
        ));
        assert!(f.pairing.registry.list().unwrap().is_empty());
    }

    #[test]
    fn approve_rejects_a_certificate_for_another_key_or_root() {
        let f = fixture();
        let now = Instant::now();
        let code = f.pairing.open("laptop", &f.csr, now).unwrap();
        let other_root = create_root_ca().unwrap();
        let foreign =
            sign_device_csr(&f.csr, &other_root.cert_pem, &other_root.key_pem, 30).unwrap();
        assert!(matches!(
            f.pairing.approve(&code, &foreign, now),
            Err(PairError::Invalid(_))
        ));
        // A certificate of the same root for a different key and id.
        let other = tempfile::tempdir().unwrap();
        init_device_key(other.path(), "01JOTHERDEVICE0000000000AA").unwrap();
        let other_csr = std::fs::read_to_string(other.path().join("csr.pem")).unwrap();
        let other_cert =
            sign_device_csr(&other_csr, &f.root.cert_pem, &f.root.key_pem, 30).unwrap();
        assert!(matches!(
            f.pairing.approve(&code, &other_cert, now),
            Err(PairError::Invalid(_))
        ));
        assert!(f.pairing.registry.list().unwrap().is_empty());
        f.pairing.approve(&code, &sign(&f), now).unwrap();
    }

    #[test]
    fn open_rejects_a_bad_request_or_name() {
        let f = fixture();
        let now = Instant::now();
        assert!(matches!(
            f.pairing.open("laptop", "nope", now),
            Err(PairError::Invalid(_))
        ));
        assert!(matches!(
            f.pairing.open("  ", &f.csr, now),
            Err(PairError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn wait_returns_at_the_approval_event_not_after_the_poll_time() {
        let f = fixture();
        let pairing = Arc::new(Pairing::with_timing(
            f.pairing.registry.clone(),
            f.root.cert_pem.clone(),
            CODE_TTL,
            Duration::from_secs(60),
        ));
        let code = pairing.open("laptop", &f.csr, Instant::now()).unwrap();
        let waiter = {
            let (pairing, code) = (pairing.clone(), code.clone());
            tokio::spawn(async move { pairing.wait(&code).await.status() })
        };
        tokio::task::yield_now().await;
        pairing.approve(&code, &sign(&f), Instant::now()).unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn wait_ends_with_204_when_no_one_approves() {
        let f = fixture();
        let pairing = Pairing::with_timing(
            f.pairing.registry.clone(),
            f.root.cert_pem.clone(),
            CODE_TTL,
            Duration::from_millis(20),
        );
        let code = pairing.open("laptop", &f.csr, Instant::now()).unwrap();
        assert_eq!(pairing.wait(&code).await.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            pairing.wait("ZZZZZZZZ").await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
