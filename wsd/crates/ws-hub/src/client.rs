//! A small mTLS client for the hub. A device uses it with the `[hub]` config section.

use std::{fs::File, io::BufReader, path::Path, sync::Arc};

use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, body::Bytes, client::conn::http1::SendRequest};
use hyper_util::rt::TokioIo;
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject},
};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::{identity::Result, server::read_certs};

pub struct HubClient {
    host: String,
    authority: String,
    connector: TlsConnector,
}

pub struct HubResponse {
    pub status: u16,
    pub body: String,
}

impl HubClient {
    /// Builds a client for `url` (`https://host:port`). The `ca` file must sign the hub
    /// server certificate.
    pub fn new(url: &str, cert: &Path, key: &Path, ca: &Path) -> Result<Self> {
        let key = PrivateKeyDer::from_pem_reader(&mut BufReader::new(File::open(key)?))?;
        Self::build(url, Some((read_certs(cert)?, key)), Some(ca))
    }

    /// Builds a client with no client certificate, for a device that is not paired yet.
    /// Without `ca` the client trusts the system roots.
    pub fn anonymous(url: &str, ca: Option<&Path>) -> Result<Self> {
        Self::build(url, None, ca)
    }

    fn build(
        url: &str,
        identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
        ca: Option<&Path>,
    ) -> Result<Self> {
        let authority = url
            .strip_prefix("https://")
            .ok_or("The hub URL must start with https://.")?
            .trim_end_matches('/')
            .to_owned();
        let host = match authority.rsplit_once(':') {
            Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host.to_owned(),
            _ => authority.clone(),
        };
        let authority = if authority.contains(':') && !authority.ends_with(']') {
            authority
        } else {
            format!("{authority}:443")
        };
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut roots = RootCertStore::empty();
        match ca {
            Some(ca) => {
                for root in read_certs(ca)? {
                    roots.add(root)?;
                }
            }
            None => {
                let native = rustls_native_certs::load_native_certs();
                roots.add_parsable_certificates(native.certs);
            }
        }
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots);
        let config = match identity {
            Some((chain, key)) => builder.with_client_auth_cert(chain, key)?,
            None => builder.with_no_client_auth(),
        };
        Ok(Self {
            host,
            authority,
            connector: TlsConnector::from(Arc::new(config)),
        })
    }

    /// One request on a new connection.
    pub async fn request(&self, method: Method, path: &str) -> Result<HubResponse> {
        self.connect().await?.request(method, path).await
    }

    /// One request with a JSON body on a new connection.
    pub async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: &str,
    ) -> Result<HubResponse> {
        self.connect()
            .await?
            .send(method, path, body.as_bytes().to_vec())
            .await
    }

    pub async fn post_json(&self, path: &str, body: Vec<u8>) -> Result<HubResponse> {
        self.connect().await?.post_json(path, body).await
    }

    /// Opens a keep-alive connection for several requests.
    pub async fn connect(&self) -> Result<HubConnection> {
        let tcp = TcpStream::connect(&self.authority).await?;
        let name = ServerName::try_from(self.host.clone())?;
        let tls = self.connector.connect(name, tcp).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await?;
        tokio::spawn(connection);
        Ok(HubConnection {
            authority: self.authority.clone(),
            sender,
        })
    }
}

pub struct HubConnection {
    authority: String,
    sender: SendRequest<Full<Bytes>>,
}

impl HubConnection {
    pub async fn request(&mut self, method: Method, path: &str) -> Result<HubResponse> {
        self.send(method, path, Vec::new()).await
    }

    pub async fn get(&mut self, path: &str) -> Result<HubResponse> {
        self.send(Method::GET, path, Vec::new()).await
    }

    pub async fn post_json(&mut self, path: &str, body: Vec<u8>) -> Result<HubResponse> {
        self.send(Method::POST, path, body).await
    }

    pub async fn send(&mut self, method: Method, path: &str, body: Vec<u8>) -> Result<HubResponse> {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", &self.authority)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))?;
        let response = self.sender.send_request(request).await?;
        let status = response.status().as_u16();
        let body = response.into_body().collect().await?.to_bytes();
        Ok(HubResponse {
            status,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }
}
