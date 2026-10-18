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
    #[cfg(test)]
    pass: Option<tokio::sync::mpsc::UnboundedSender<bool>>,
    #[cfg(test)]
    retry: Option<Arc<tokio::sync::Notify>>,
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
                        let result =
                            deliver(&client, &sink, &log, &store, retention_ms, &options).await;
                        #[cfg(test)]
                        if let Some(pass) = &options.pass {
                            let _ = pass.send(result.is_ok());
                        }
                        #[cfg(test)]
                        if result.is_err()
                            && let Some(retry) = &options.retry
                        {
                            retry.notified().await;
                            continue;
                        }
                        let _ = result;
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
            #[cfg(test)]
            pass: None,
            #[cfg(test)]
            retry: None,
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
                        if let Some(attributes) = item["attributes"].as_array_mut()
                            && !attributes.iter().any(|a| a["key"] == "event_id")
                        {
                            attributes.push(json!({"key": "event_id", "value": {"stringValue": record.event_id.to_string()}}));
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

    struct TestSink {
        address: std::net::SocketAddr,
        task: Option<JoinHandle<()>>,
    }

    struct Request {
        path: String,
        payload: Value,
        reply: tokio::sync::oneshot::Sender<Option<Value>>,
    }

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
    #[tokio::test]
    async fn sink_restart_loses_no_records() {
        let sandbox = tempfile::tempdir().unwrap();
        let (sink, mut received) = TestSink::bind(None).await;
        let address = sink.address;
        sink.stop().await;
        configure(sandbox.path(), &[format!("http://{address}")]);
        let log = test_log(sandbox.path());
        append_log(&log, "before downtime", None);
        let (options, mut passes, retry) = controlled();
        let forwarder = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        assert!(!receive(&mut passes).await);
        append_log(&log, "during downtime", None);
        let (sink, replacement) = TestSink::bind(Some(address)).await;
        received.close();
        received = replacement;
        retry.notify_one();
        let request = receive(&mut received).await;
        assert_eq!(request.path, "/v1/logs");
        assert_eq!(
            bodies(&request.payload),
            ["before downtime", "during downtime"]
        );
        request.reply.send(Some(json!({}))).unwrap();
        assert!(receive(&mut passes).await);
        drop(forwarder);
        sink.stop().await;
    }

    #[tokio::test]
    async fn accepted_batch_with_dropped_response_repeats_with_the_same_log_and_span_ids() {
        let sandbox = tempfile::tempdir().unwrap();
        let (sink, mut received) = TestSink::bind(None).await;
        configure(sandbox.path(), &[sink.url()]);
        let log = test_log(sandbox.path());
        append_log(&log, "source", Some("source-event"));
        append_span(&log);
        let (options, mut passes, retry) = controlled();
        let forwarder = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        let first = receive(&mut received).await;
        let original = first.payload.clone();
        first.reply.send(None).unwrap();
        assert!(!receive(&mut passes).await);
        retry.notify_one();
        let repeated = receive(&mut received).await;
        assert_eq!(repeated.payload, original);
        assert_eq!(log_ids(&repeated.payload), ["source-event"]);
        repeated.reply.send(Some(json!({}))).unwrap();
        let span = receive(&mut received).await;
        assert_eq!(span.path, "/v1/traces");
        assert_eq!(
            span.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["spanId"],
            "0102030405060708"
        );
        let original_span = span.payload.clone();
        span.reply.send(None).unwrap();
        assert!(!receive(&mut passes).await);
        retry.notify_one();
        let repeated_span = receive(&mut received).await;
        assert_eq!(repeated_span.payload, original_span);
        repeated_span.reply.send(Some(json!({}))).unwrap();
        assert!(receive(&mut passes).await);
        drop(forwarder);
        // An acknowledged cursor survives a restart. Only the new record is delivered.
        append_log(&log, "after restart", None);
        let (options, mut passes, _) = controlled();
        let restarted = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        let next = receive(&mut received).await;
        assert_eq!(bodies(&next.payload), ["after restart"]);
        next.reply.send(Some(json!({}))).unwrap();
        assert!(receive(&mut passes).await);
        drop(restarted);
        sink.stop().await;
    }

    #[tokio::test]
    async fn process_crash_after_ack_before_cursor_commit_replays_the_batch() {
        let sandbox = tempfile::tempdir().unwrap();
        let (sink, mut received) = TestSink::bind(None).await;
        configure(sandbox.path(), &[sink.url()]);
        let log = test_log(sandbox.path());
        append_log(&log, "crash boundary", None);
        let gate_path = sandbox.path().join("commit.sock");
        let gate = tokio::net::UnixListener::bind(&gate_path).unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "forwarder::tests::crash_child", "--nocapture"])
            .env("P45_CHILD_STATE", sandbox.path())
            .env("P45_CHILD_GATE", &gate_path)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let first = receive(&mut received).await;
        let original = first.payload.clone();
        first.reply.send(Some(json!({}))).unwrap();
        // The child announces this boundary after it reads the acknowledgement.
        let _connection = tokio::time::timeout(Duration::from_secs(30), gate.accept())
            .await
            .unwrap()
            .unwrap();
        child.kill().await.unwrap();
        let (options, mut passes, _) = controlled();
        let restarted = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        let replay = receive(&mut received).await;
        assert_eq!(replay.payload, original);
        replay.reply.send(Some(json!({}))).unwrap();
        assert!(receive(&mut passes).await);
        drop(restarted);
        sink.stop().await;
    }

    #[tokio::test]
    async fn crash_child() {
        let Some(state) = std::env::var_os("P45_CHILD_STATE") else {
            return;
        };
        let gate = std::env::var_os("P45_CHILD_GATE").unwrap();
        let options = Options {
            before_commit: Some(Arc::new(move || {
                let _connection = std::os::unix::net::UnixStream::connect(&gate).unwrap();
                loop {
                    std::thread::park();
                }
            })),
            ..Options::default()
        };
        let _forwarder = Forwarder::start_with_options(Path::new(&state), options).unwrap();
        std::future::pending::<()>().await;
    }

    #[tokio::test]
    async fn sink_cursors_are_independent_per_device_and_new_sinks_replay_retention() {
        let sandbox = tempfile::tempdir().unwrap();
        let (blocked, mut blocked_requests) = TestSink::bind(None).await;
        let (healthy, mut healthy_requests) = TestSink::bind(None).await;
        configure(sandbox.path(), &[blocked.url(), healthy.url()]);
        let local = test_log(sandbox.path());
        let remote = TelemetryLog::new(sandbox.path(), ws_core::ids::Ulid::new());
        append_log(&local, "local old", None);
        append_log(&remote, "remote old", None);
        let expired = json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": "expired"}}]}]}]});
        local.append(Signal::Logs, vec![expired], 0).unwrap();
        let (options, mut passes, _) = controlled();
        let forwarder = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        // Hold one sink's response while the other sink acknowledges both devices.
        let held = receive(&mut blocked_requests).await;
        let mut seen = Vec::new();
        for _ in 0..2 {
            let request = receive(&mut healthy_requests).await;
            seen.extend(bodies(&request.payload));
            request.reply.send(Some(json!({}))).unwrap();
        }
        seen.sort();
        assert_eq!(seen, ["local old", "remote old"]);
        assert!(receive(&mut passes).await);
        drop(forwarder);
        drop(held);
        blocked.stop().await;
        let (new, mut new_requests) = TestSink::bind(None).await;
        // Sink order changes. The healthy sink retains both device cursors.
        configure(sandbox.path(), &[new.url(), healthy.url()]);
        append_log(&local, "local new", None);
        append_log(&remote, "remote new", None);
        let (options, mut passes, _) = controlled();
        let restarted = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        let mut incremental = Vec::new();
        let mut replay = Vec::new();
        for _ in 0..2 {
            let request = receive(&mut healthy_requests).await;
            incremental.extend(bodies(&request.payload));
            request.reply.send(Some(json!({}))).unwrap();
            let request = receive(&mut new_requests).await;
            replay.extend(bodies(&request.payload));
            request.reply.send(Some(json!({}))).unwrap();
        }
        incremental.sort();
        replay.sort();
        assert_eq!(incremental, ["local new", "remote new"]);
        assert_eq!(
            replay,
            ["local new", "local old", "remote new", "remote old"]
        );
        assert!(receive(&mut passes).await);
        assert!(receive(&mut passes).await);
        drop(restarted);
        healthy.stop().await;
        new.stop().await;
    }

    #[tokio::test]
    async fn partial_rejection_keeps_the_cursor_and_metric_identity_for_retry() {
        let sandbox = tempfile::tempdir().unwrap();
        let (sink, mut received) = TestSink::bind(None).await;
        configure(sandbox.path(), &[sink.url()]);
        let log = test_log(sandbox.path());
        log.append(Signal::Metrics, vec![json!({"resourceMetrics": [{"scopeMetrics": [{"metrics": [{"name": "tokens", "sum": {"dataPoints": [{"asInt": "12", "timeUnixNano": "1000000000"}]}}]}]}]})], now_ms()).unwrap();
        let (options, mut passes, retry) = controlled();
        let forwarder = Forwarder::start_with_options(sandbox.path(), options).unwrap();
        let first = receive(&mut received).await;
        assert_eq!(first.path, "/v1/metrics");
        let original = first.payload.clone();
        first
            .reply
            .send(Some(json!({"partialSuccess": {"rejectedDataPoints": "1"}})))
            .unwrap();
        assert!(!receive(&mut passes).await);
        retry.notify_one();
        let replay = receive(&mut received).await;
        assert_eq!(replay.payload, original);
        replay
            .reply
            .send(Some(json!({"partialSuccess": {"rejectedDataPoints": "0"}})))
            .unwrap();
        assert!(receive(&mut passes).await);
        drop(forwarder);
        sink.stop().await;
    }

    impl TestSink {
        async fn bind(
            address: Option<std::net::SocketAddr>,
        ) -> (Self, mpsc::UnboundedReceiver<Request>) {
            use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
            let listener =
                TcpListener::bind(address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()))
                    .await
                    .unwrap();
            let address = listener.local_addr().unwrap();
            let (tx, rx) = mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let path = line.split_whitespace().nth(1).unwrap().to_owned();
                    let mut length = 0;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).await.unwrap();
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    let mut bytes = vec![0; length];
                    reader.read_exact(&mut bytes).await.unwrap();
                    let (reply, response) = tokio::sync::oneshot::channel();
                    if tx
                        .send(Request {
                            path,
                            payload: serde_json::from_slice(&bytes).unwrap(),
                            reply,
                        })
                        .is_err()
                    {
                        return;
                    }
                    if let Ok(Some(response)) = response.await {
                        let body = response.to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = reader.get_mut().write_all(response.as_bytes()).await;
                    }
                }
            });
            (
                Self {
                    address,
                    task: Some(task),
                },
                rx,
            )
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }

        async fn stop(mut self) {
            let task = self.task.take().unwrap();
            task.abort();
            let _ = task.await;
        }
    }

    impl Drop for TestSink {
        fn drop(&mut self) {
            if let Some(task) = &self.task {
                task.abort();
            }
        }
    }

    fn configure(state: &Path, sinks: &[String]) {
        fs::write(
            state.join("config.toml"),
            format!(
                "[telemetry]\nsinks = {}\n",
                serde_json::to_string(sinks).unwrap()
            ),
        )
        .unwrap();
    }

    fn test_log(state: &Path) -> TelemetryLog {
        TelemetryLog::new(state, Device::init(state, "test-device").unwrap().device_id)
    }

    fn append_log(log: &TelemetryLog, body: &str, id: Option<&str>) {
        let attributes = id
            .map(|id| vec![json!({"key": "event_id", "value": {"stringValue": id}})])
            .unwrap_or_default();
        log.append(Signal::Logs, vec![json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": body}, "attributes": attributes}]}]}]})], now_ms()).unwrap();
    }

    fn append_span(log: &TelemetryLog) {
        log.append(Signal::Traces, vec![json!({"resourceSpans": [{"scopeSpans": [{"spans": [{"name": "turn", "traceId": "01020304050607080910111213141516", "spanId": "0102030405060708"}]}]}]})], now_ms()).unwrap();
    }

    fn bodies(payload: &Value) -> Vec<String> {
        payload["resourceLogs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["scopeLogs"].as_array().unwrap())
            .flat_map(|s| s["logRecords"].as_array().unwrap())
            .map(|r| r["body"]["stringValue"].as_str().unwrap().to_owned())
            .collect()
    }

    fn log_ids(payload: &Value) -> Vec<String> {
        payload["resourceLogs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["scopeLogs"].as_array().unwrap())
            .flat_map(|s| s["logRecords"].as_array().unwrap())
            .flat_map(|r| r["attributes"].as_array().unwrap())
            .filter(|a| a["key"] == "event_id")
            .map(|a| a["value"]["stringValue"].as_str().unwrap().to_owned())
            .collect()
    }

    fn controlled() -> (
        Options,
        mpsc::UnboundedReceiver<bool>,
        Arc<tokio::sync::Notify>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let retry = Arc::new(tokio::sync::Notify::new());
        (
            Options {
                interval: Duration::from_secs(3600),
                pass: Some(tx),
                retry: Some(retry.clone()),
                ..Options::default()
            },
            rx,
            retry,
        )
    }

    async fn receive<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
        tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .unwrap()
            .unwrap()
    }
}
