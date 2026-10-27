//! The hub HTTPS server. Each `/v1/` route needs a client certificate that chains to
//! the WS root CA and belongs to an unrevoked device of the registry.

use std::{
    convert::Infallible,
    fs::File,
    io::BufReader,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode, body::Bytes, body::Incoming, server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::WebPkiClientVerifier,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::{
    identity::Result,
    registry::{Registry, der_fingerprint},
    sync::{SyncBatch, SyncStore},
};

/// The largest accepted request body.
const MAX_BODY_BYTES: usize = 8 << 20;

pub struct HubOptions {
    pub listen: SocketAddr,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    pub ca_cert: PathBuf,
    pub state: PathBuf,
}

pub struct Hub {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    registry: Arc<Registry>,
    sync: Arc<SyncStore>,
}

/// Binds the listener and loads the certificates and the registry.
pub async fn bind(options: &HubOptions) -> Result<Hub> {
    let registry = Arc::new(Registry::open(&options.state)?);
    let acceptor = TlsAcceptor::from(Arc::new(tls_config(options)?));
    let sync = Arc::new(SyncStore::open(&options.state)?);
    let listener = TcpListener::bind(options.listen).await?;
    Ok(Hub {
        listener,
        acceptor,
        registry,
        sync,
    })
}

impl Hub {
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    /// Accepts connections until the task ends.
    pub async fn serve(self) -> Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let acceptor = self.acceptor.clone();
            let registry = self.registry.clone();
            let sync = self.sync.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else {
                    return;
                };
                let fingerprint = tls
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|chain| chain.first())
                    .map(|cert| der_fingerprint(cert));
                let service = service_fn(move |request| {
                    let registry = registry.clone();
                    let sync = sync.clone();
                    let fingerprint = fingerprint.clone();
                    async move {
                        Ok::<_, Infallible>(
                            route(&registry, &sync, fingerprint.as_deref(), request).await,
                        )
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    }
}

async fn route(
    registry: &Registry,
    sync: &SyncStore,
    fingerprint: Option<&str>,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    if path.starts_with("/v1/pair/") || path == "/mcp" {
        return reply(StatusCode::NOT_FOUND, "Not implemented.");
    }
    if !path.starts_with("/v1/") {
        return reply(StatusCode::NOT_FOUND, "Unknown route.");
    }
    // The registry check runs on each request, so a revoked device fails on a kept connection.
    let device = match fingerprint.map(|print| registry.active_by_fingerprint(print)) {
        Some(Ok(Some(device))) => device,
        Some(Err(error)) => return reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        _ => return reply(StatusCode::FORBIDDEN, "Device not allowed."),
    };
    match (request.method(), path.as_str()) {
        (&Method::GET, "/v1/devices") => match registry.list() {
            Ok(rows) => json(StatusCode::OK, &rows),
            Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        (&Method::DELETE, _) if path.starts_with("/v1/devices/") => {
            match registry.revoke(&path["/v1/devices/".len()..]) {
                Ok(true) => reply(StatusCode::NO_CONTENT, ""),
                Ok(false) => reply(StatusCode::NOT_FOUND, "No such device."),
                Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
            }
        }
        (&Method::GET, "/v1/sync/cursors") => match sync.cursors(&device.id) {
            Ok(cursors) => json(StatusCode::OK, &cursors),
            Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        (&Method::POST, "/v1/sync/logs") => ingest_logs(sync, &device.id, request).await,
        _ => reply(StatusCode::NOT_FOUND, "Unknown route."),
    }
}

/// Stores a batch for the device of the client certificate. The body names no device.
async fn ingest_logs(
    sync: &SyncStore,
    device_id: &str,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let Ok(body) = Limited::new(request.into_body(), MAX_BODY_BYTES).collect().await else {
        return reply(StatusCode::PAYLOAD_TOO_LARGE, "Cannot read the request body.");
    };
    let Ok(batch) = serde_json::from_slice::<SyncBatch>(&body.to_bytes()) else {
        return reply(StatusCode::BAD_REQUEST, "The batch is not valid JSON.");
    };
    match sync.ingest(device_id, &batch) {
        Ok(stored) => json(StatusCode::OK, &serde_json::json!({ "stored": stored })),
        Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

fn tls_config(options: &HubOptions) -> Result<ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = RootCertStore::empty();
    for cert in read_certs(&options.ca_cert)? {
        roots.add(cert)?;
    }
    // Pairing and MCP routes take no client certificate, so the handshake allows none.
    // The `/v1/` routes reject a missing certificate.
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .allow_unauthenticated()
        .build()?;
    let key = PrivateKeyDer::from_pem_file(&options.tls_key)?;
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(read_certs(&options.tls_cert)?, key)?;
    Ok(config)
}

pub fn read_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = BufReader::new(File::open(path)?);
    Ok(CertificateDer::pem_reader_iter(&mut reader).collect::<std::result::Result<_, _>>()?)
}

fn json<T: serde::Serialize>(status: StatusCode, value: &T) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(
        serde_json::to_vec(value).unwrap_or_default(),
    )));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    response
}

fn reply(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(message.to_owned())));
    *response.status_mut() = status;
    response
}
