//! At-least-once OTLP/HTTP export from the durable telemetry log.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use ws_core::{
    config::Config,
    device::Device,
    telemetry_log::{Signal, TelemetryLog, TelemetryRecord},
};

const BATCH_SIZE: usize = 64;

#[derive(Default, Clone, Deserialize, Serialize)]
struct Cursors(BTreeMap<String, BTreeMap<String, String>>);

struct CursorStore {
    path: PathBuf,
    cursors: Cursors,
}

#[derive(Clone)]
struct Options {
    interval: Duration,
    #[cfg(test)]
    before_commit: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Owns one independent delivery worker per configured sink.
pub struct Forwarder {
    tasks: Vec<JoinHandle<()>>,
}

impl Forwarder {
    pub fn start(state: &Path) -> io::Result<Self> {
        Self::start_with_options(state, Options::default())
    }

    fn start_with_options(state: &Path, options: Options) -> io::Result<Self> {
        let config = Config::load(state)?.telemetry;
        let device = Device::load(state)?;
        let log = TelemetryLog::new(state, device.device_id);
        let path = state.join("telemetry-cursors.json");
        let cursors = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Cursors::default(),
            Err(error) => return Err(error),
        };
        let store = Arc::new(Mutex::new(CursorStore { path, cursors }));
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(io::Error::other)?;
        let mut sinks = Vec::new();
        for sink in config.sinks {
            let mut url = Url::parse(&sink).map_err(io::Error::other)?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The telemetry sink must be an HTTP or HTTPS base URL without a query or fragment.",
                ));
            }
            let path = format!("{}/", url.path().trim_end_matches('/'));
            url.set_path(&path);
            if !sinks.contains(&url) {
                sinks.push(url);
            }
        }
        let tasks = sinks
            .into_iter()
            .map(|sink| {
                let client = client.clone();
                let log = log.clone();
                let store = store.clone();
                let options = options.clone();
                let retention_ms = config.retention_days.saturating_mul(86_400_000);
                tokio::spawn(async move {
                    loop {
                        // A failed sink cannot block another sink. The durable log is the retry queue.
                        let _ = deliver(&client, &sink, &log, &store, retention_ms, &options).await;
                        tokio::time::sleep(options.interval).await;
                    }
                })
            })
            .collect();
        Ok(Self { tasks })
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            #[cfg(test)]
            before_commit: None,
        }
    }
}

async fn deliver(
    client: &Client,
    sink: &Url,
    log: &TelemetryLog,
    store: &Arc<Mutex<CursorStore>>,
    retention_ms: u64,
    options: &Options,
) -> io::Result<()> {
    let read_log = log.clone();
    let records = tokio::task::spawn_blocking(move || read_log.read())
        .await
        .map_err(io::Error::other)??;
    let mut devices: BTreeMap<String, Vec<TelemetryRecord>> = BTreeMap::new();
    let cutoff = now_ms().saturating_sub(retention_ms);
    for record in records.into_iter().filter(|r| r.received_at_ms >= cutoff) {
        let device = source_device(&record)?;
        devices.entry(device).or_default().push(record);
    }
    let cursors = store
        .lock()
        .unwrap()
        .cursors
        .0
        .get(sink.as_str())
        .cloned()
        .unwrap_or_default();
    for (device, records) in devices {
        // Locate by log order, never by ULID order: concurrent ingestion can reorder IDs.
        // If retention removed the cursor record, replay the retained window.
        let mut index = cursors
            .get(&device)
            .and_then(|cursor| {
                records
                    .iter()
                    .position(|r| r.event_id.to_string() == *cursor)
            })
            .map_or(0, |i| i + 1);
        while index < records.len() {
            let signal = records[index].signal;
            let end = records[index..]
                .iter()
                .take(BATCH_SIZE)
                .take_while(|r| r.signal == signal)
                .count()
                + index;
            let payload = batch_payload(&records[index..end]);
            let route = match signal {
                Signal::Logs => "v1/logs",
                Signal::Metrics => "v1/metrics",
                Signal::Traces => "v1/traces",
            };
            let response = client
                .post(sink.join(route).map_err(io::Error::other)?)
                .json(&payload)
                .send()
                .await
                .map_err(io::Error::other)?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(io::Error::other(
                    "The telemetry sink did not acknowledge the batch.",
                ));
            }
            let response: Value = response.json().await.map_err(io::Error::other)?;
            if !response.is_object()
                || response.get("partialSuccess").is_some_and(|partial| {
                    ["rejectedLogRecords", "rejectedSpans", "rejectedDataPoints"]
                        .iter()
                        .any(|key| partial.get(key).is_some_and(|v| v != 0 && v != "0"))
                })
            {
                return Err(io::Error::other(
                    "The telemetry sink rejected records in the batch.",
                ));
            }
            #[cfg(test)]
            if let Some(before_commit) = &options.before_commit {
                before_commit();
            }
            #[cfg(not(test))]
            let _ = options;
            let store = store.clone();
            let sink = sink.to_string();
            let device = device.clone();
            let cursor = records[end - 1].event_id.to_string();
            tokio::task::spawn_blocking(move || store.lock().unwrap().commit(sink, device, cursor))
                .await
                .map_err(io::Error::other)??;
            index = end;
        }
    }
    Ok(())
}

impl CursorStore {
    fn commit(&mut self, sink: String, device: String, cursor: String) -> io::Result<()> {
        let mut next = self.cursors.clone();
        next.0.entry(sink).or_default().insert(device, cursor);
        let directory = self.path.parent().unwrap();
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut temporary, &next)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|e| e.error)?;
        File::open(directory)?.sync_all()?;
        self.cursors = next;
        Ok(())
    }
}

fn source_device(record: &TelemetryRecord) -> io::Result<String> {
    record.payload[record.signal.resource_key()][0]["resource"]["attributes"]
        .as_array()
        .and_then(|attrs| attrs.iter().find(|a| a["key"] == "ws.device.id"))
        .and_then(|a| a["value"]["stringValue"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("The telemetry record has no source device ID."))
}

fn batch_payload(records: &[TelemetryRecord]) -> Value {
    let signal = records[0].signal;
    let mut resources = Vec::new();
    for record in records {
        let mut payload = record.payload.clone();
        if signal == Signal::Logs {
            for resource in payload["resourceLogs"].as_array_mut().into_iter().flatten() {
                for scope in resource["scopeLogs"].as_array_mut().into_iter().flatten() {
                    for item in scope["logRecords"].as_array_mut().into_iter().flatten() {
                        if item.get("attributes").is_none_or(Value::is_null) {
                            item["attributes"] = json!([]);
                        }
                        if let Some(attributes) = item["attributes"].as_array_mut() {
                            if !attributes.iter().any(|a| a["key"] == "event_id") {
                                attributes.push(json!({"key": "event_id", "value": {"stringValue": record.event_id.to_string()}}));
                            }
                        }
                    }
                }
            }
        }
        resources.extend(
            payload[signal.resource_key()]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    json!({signal.resource_key(): resources})
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    use std::{fs, future::IntoFuture};
    use tokio::{net::TcpListener, sync::mpsc};
    use ws_core::{
        device::Device,
        telemetry_log::{Signal, TelemetryLog},
    };

    #[tokio::test]
    async fn every_sink_receives_retained_records_with_stable_identities() {
        let sandbox = tempfile::tempdir().unwrap();
        let device = Device::init(sandbox.path(), "test-device").unwrap();
        let log = TelemetryLog::new(sandbox.path(), device.device_id);
        let records = log.append(Signal::Logs, vec![json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": "retained"}}]}]}]})], now_ms()).unwrap();
        let mut sinks = Vec::new();
        let mut tasks = Vec::new();
        let mut receivers = Vec::new();
        for _ in 0..2 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            sinks.push(format!("http://{}", listener.local_addr().unwrap()));
            let (tx, rx) = mpsc::unbounded_channel();
            receivers.push(rx);
            let router = Router::new().route(
                "/v1/logs",
                post(move |Json(payload): Json<Value>| async move {
                    tx.send(payload).unwrap();
                    Json(json!({}))
                }),
            );
            tasks.push(tokio::spawn(axum::serve(listener, router).into_future()));
        }
        fs::write(
            sandbox.path().join("config.toml"),
            format!(
                "[telemetry]\nsinks = {}\n",
                serde_json::to_string(&sinks).unwrap()
            ),
        )
        .unwrap();
        let forwarder = Forwarder::start(sandbox.path()).unwrap();
        for rx in &mut receivers {
            let payload = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let record = &payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
            assert_eq!(record["body"]["stringValue"], "retained");
            assert!(
                record["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["key"] == "event_id"
                        && a["value"]["stringValue"] == records[0].event_id.to_string())
            );
        }
        drop(forwarder);
        for task in tasks {
            task.abort();
        }
    }
}
