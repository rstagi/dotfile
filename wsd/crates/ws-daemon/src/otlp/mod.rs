//! Local OTLP/HTTP ingestion.

use std::{
    fs,
    future::IntoFuture,
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
    trace::v1::ExportTraceServiceRequest,
};
use prost::Message;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use ws_core::{
    config::Config,
    device::Device,
    telemetry_log::{Signal, TelemetryLog},
};

#[derive(Clone, Copy)]
enum Encoding {
    Json,
    Protobuf,
}

#[derive(Clone)]
struct Ingestion {
    log: TelemetryLog,
    content: bool,
}

struct EndpointFile {
    path: PathBuf,
    device: u64,
    inode: u64,
}

pub struct Receiver {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<io::Result<()>>>,
    _endpoint: EndpointFile,
}

impl Receiver {
    /// Binds only to IPv4 localhost. The listener is ready when this function returns.
    pub async fn bind(state: &Path) -> io::Result<Self> {
        let config = Config::load(state)?.telemetry;
        let device = Device::load(state)?;
        let log = TelemetryLog::new(state, device.device_id);
        let retention_ms = config.retention_days.saturating_mul(86_400_000);
        prune(log.clone(), retention_ms).await?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).await?;
        let address = listener.local_addr()?;
        let endpoint = publish_endpoint(state, address)?;
        let maintenance_log = log.clone();
        let app = Router::new()
            .route("/v1/logs", post(logs))
            .route("/v1/metrics", post(metrics))
            .route("/v1/traces", post(traces))
            .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
            .with_state(Ingestion {
                log,
                content: config.content,
            });
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .into_future();
            tokio::pin!(server);
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await;
            loop {
                tokio::select! {
                    result = &mut server => return result,
                    _ = interval.tick() => prune(maintenance_log.clone(), retention_ms).await?,
                }
            }
        });
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            task: Some(task),
            _endpoint: endpoint,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Reports a receiver failure so that the daemon can stop with an error.
    pub async fn wait(&mut self) -> io::Result<()> {
        let Some(task) = self.task.as_mut() else {
            return Ok(());
        };
        let result = task.await;
        self.task.take();
        result.map_err(io::Error::other)?
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.wait().await
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for EndpointFile {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn publish_endpoint(state: &Path, address: SocketAddr) -> io::Result<EndpointFile> {
    let path = state.join("otlp.endpoint");
    let mut temporary = tempfile::NamedTempFile::new_in(state)?;
    writeln!(temporary, "http://{address}")?;
    temporary.as_file().sync_all()?;
    let file = temporary.persist(&path).map_err(|error| error.error)?;
    let metadata = file.metadata()?;
    let endpoint = EndpointFile {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    fs::File::open(state)?.sync_all()?;
    Ok(endpoint)
}

async fn prune(log: TelemetryLog, retention_ms: u64) -> io::Result<()> {
    tokio::task::spawn_blocking(move || log.prune_before(now_ms().saturating_sub(retention_ms)))
        .await
        .map_err(io::Error::other)??;
    Ok(())
}

async fn logs(State(state): State<Ingestion>, headers: HeaderMap, body: Bytes) -> Response {
    ingest(state, Signal::Logs, headers, body).await
}

async fn metrics(State(state): State<Ingestion>, headers: HeaderMap, body: Bytes) -> Response {
    ingest(state, Signal::Metrics, headers, body).await
}

async fn traces(State(state): State<Ingestion>, headers: HeaderMap, body: Bytes) -> Response {
    ingest(state, Signal::Traces, headers, body).await
}

async fn ingest(state: Ingestion, signal: Signal, headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .trim();
    let encoding = if content_type.eq_ignore_ascii_case("application/json") {
        Encoding::Json
    } else if content_type.eq_ignore_ascii_case("application/x-protobuf") {
        Encoding::Protobuf
    } else {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Use application/json or application/x-protobuf.",
        )
            .into_response();
    };
    let payload = match signal {
        Signal::Logs => decode::<ExportLogsServiceRequest>(&body, encoding),
        Signal::Metrics => decode::<ExportMetricsServiceRequest>(&body, encoding),
        Signal::Traces => decode::<ExportTraceServiceRequest>(&body, encoding),
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(()) => {
            return (StatusCode::BAD_REQUEST, "The OTLP request body is invalid.").into_response();
        }
    };
    let mut payloads = split_records(payload, signal);
    if signal == Signal::Traces {
        payloads.retain(|payload| !crate::spans::is_native_boundary(payload));
    }
    for payload in &mut payloads {
        redact(payload, state.content, false);
    }
    let log = state.log;
    match tokio::task::spawn_blocking(move || log.append(signal, payloads, now_ms())).await {
        Ok(Ok(_)) => match encoding {
            Encoding::Json => ([("content-type", "application/json")], "{}").into_response(),
            // All three Export*ServiceResponse messages encode an empty success as zero bytes.
            Encoding::Protobuf => (
                [("content-type", "application/x-protobuf")],
                Vec::<u8>::new(),
            )
                .into_response(),
        },
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot store the telemetry records.",
        )
            .into_response(),
    }
}

fn decode<T: Message + Default + DeserializeOwned + Serialize>(
    body: &[u8],
    encoding: Encoding,
) -> Result<Value, ()> {
    let request: T = match encoding {
        Encoding::Json => serde_json::from_slice(body).map_err(|_| ())?,
        Encoding::Protobuf => T::decode(body).map_err(|_| ())?,
    };
    serde_json::to_value(request).map_err(|_| ())
}

fn split_records(mut payload: Value, signal: Signal) -> Vec<Value> {
    let (scope_key, record_key) = match signal {
        Signal::Logs => ("scopeLogs", "logRecords"),
        Signal::Metrics => ("scopeMetrics", "metrics"),
        Signal::Traces => ("scopeSpans", "spans"),
    };
    let mut records = Vec::new();
    let Some(resources) = payload[signal.resource_key()].as_array_mut() else {
        return records;
    };
    for mut resource in std::mem::take(resources) {
        let Some(scopes) = resource[scope_key].as_array_mut() else {
            continue;
        };
        for mut scope in std::mem::take(scopes) {
            let Some(items) = scope[record_key].as_array_mut() else {
                continue;
            };
            for item in std::mem::take(items) {
                let mut scope = scope.clone();
                scope[record_key] = json!([item]);
                let mut resource = resource.clone();
                resource[scope_key] = json!([scope]);
                records.push(json!({ signal.resource_key(): [resource] }));
            }
        }
    }
    records
}

// Remove private attributes before the append-only log receives a record.
fn redact(value: &mut Value, content: bool, tool_result: bool) {
    match value {
        Value::Object(object) => {
            let tool_result = tool_result
                || object
                    .get("attributes")
                    .and_then(Value::as_array)
                    .is_some_and(|attributes| {
                        attributes.iter().any(|a| {
                            a["key"] == "event.name"
                                && a["value"]["stringValue"] == "codex.tool_result"
                        })
                    });
            object.remove("user.email");
            for child in object.values_mut() {
                redact(child, content, tool_result);
            }
        }
        Value::Array(values) => {
            values.retain(|value| {
                value["key"] != "user.email"
                    && (content
                        || !tool_result
                        || !matches!(value["key"].as_str(), Some("arguments" | "output")))
            });
            for child in values {
                redact(child, content, tool_result);
            }
        }
        _ => {}
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
