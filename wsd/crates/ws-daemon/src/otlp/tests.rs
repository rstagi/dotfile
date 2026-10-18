use std::{net::SocketAddr, path::Path, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use ws_core::{
    device::Device,
    semconv,
    telemetry_log::{Signal, TelemetryLog},
};

use super::Receiver;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/spikes/s2/");

#[tokio::test]
async fn accepts_the_claude_json_logs_and_persists_every_record() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    assert_eq!(receiver.local_addr().ip().to_string(), "127.0.0.1");
    assert_ne!(receiver.local_addr().port(), 0);
    let response = post(
        receiver.local_addr(),
        "/v1/logs",
        "application/json",
        &std::fs::read(Path::new(FIXTURES).join("baseline-logs.json")).unwrap(),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("application/json"), "{response}");
    let records = TelemetryLog::new(sandbox.path(), device.device_id)
        .read()
        .unwrap();
    assert_eq!(records.len(), 11);
    for record in &records {
        assert_eq!(record.signal, Signal::Logs);
        let resources = record.payload["resourceLogs"].as_array().unwrap();
        assert_eq!(resources.len(), 1);
        let attributes = resources[0]["resource"]["attributes"].as_array().unwrap();
        assert!(attributes.iter().any(|a| a["key"] == semconv::DEVICE_ID
            && a["value"]["stringValue"] == device.device_id.to_string()));
        assert!(attributes.iter().any(|a| a["key"] == semconv::SESSION_ID
            && a["value"]["stringValue"] == "s2-synthetic-session"));
        assert_eq!(
            resources[0]["scopeLogs"][0]["logRecords"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    receiver.shutdown().await.unwrap();
    let persisted = TelemetryLog::new(sandbox.path(), device.device_id)
        .read()
        .unwrap();
    assert_eq!(persisted, records);
}

#[tokio::test]
async fn accepts_all_s2_signals_in_both_wire_formats() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    let cases = [
        ("baseline-logs.json", Signal::Logs, 11),
        ("baseline-metrics.json", Signal::Metrics, 3),
        ("beta-json-traces.json", Signal::Traces, 1),
        ("codex-baseline-logs.json", Signal::Logs, 24),
        ("codex-baseline-metrics.json", Signal::Metrics, 16),
        ("codex-baseline-traces.json", Signal::Traces, 29),
        ("beta-protobuf-logs.pb", Signal::Logs, 7),
        ("beta-protobuf-metrics.pb", Signal::Metrics, 3),
        ("beta-protobuf-traces.pb", Signal::Traces, 1),
        ("codex-protobuf-logs.pb", Signal::Logs, 24),
        ("codex-protobuf-metrics.pb", Signal::Metrics, 16),
        ("codex-protobuf-traces.pb", Signal::Traces, 29),
    ];
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    let log = TelemetryLog::new(sandbox.path(), device.device_id);
    let mut offset = 0;
    for (name, signal, count) in cases {
        let path = match signal {
            Signal::Logs => "/v1/logs",
            Signal::Metrics => "/v1/metrics",
            Signal::Traces => "/v1/traces",
        };
        let content_type = if name.ends_with(".pb") {
            "application/x-protobuf"
        } else {
            "application/json"
        };
        let response = post(
            receiver.local_addr(),
            path,
            content_type,
            &std::fs::read(Path::new(FIXTURES).join(name)).unwrap(),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"), "{name}: {response}");
        assert!(response.contains(content_type), "{name}: {response}");
        let records = log.read().unwrap();
        assert_eq!(records.len(), offset + count, "{name}");
        for record in &records[offset..] {
            assert_eq!(record.signal, signal, "{name}");
            let resource = &record.payload[signal.resource_key()][0];
            assert!(
                resource["resource"]["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["key"] == semconv::DEVICE_ID
                        && a["value"]["stringValue"] == device.device_id.to_string()),
                "{name}"
            );
        }
        let reference_name = if name.ends_with(".pb") {
            format!("{name}.json")
        } else {
            name.to_owned()
        };
        let reference: serde_json::Value = serde_json::from_slice(
            &std::fs::read(Path::new(FIXTURES).join(reference_name)).unwrap(),
        )
        .unwrap();
        let (scope_key, record_key) = match signal {
            Signal::Logs => ("scopeLogs", "logRecords"),
            Signal::Metrics => ("scopeMetrics", "metrics"),
            Signal::Traces => ("scopeSpans", "spans"),
        };
        let expected = reference[signal.resource_key()]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r[scope_key].as_array().unwrap())
            .flat_map(|s| s[record_key].as_array().unwrap())
            .filter(|record| {
                signal != Signal::Traces
                    || !matches!(
                        record["name"].as_str(),
                        Some(
                            "claude_code.tool"
                                | "claude_code.tool.execution"
                                | "claude_code.llm_request"
                                | "run_turn"
                                | "session_task.turn"
                                | "handle_tool_call_with_source"
                                | "dispatch_tool_call_with_state"
                                | "exec_command"
                                | "unified_exec.exec_command"
                                | "code_mode.broker.invoke_tool"
                                | "exec"
                        )
                    )
            });
        for (stored, original) in records[offset..].iter().zip(expected) {
            let item = &stored.payload[signal.resource_key()][0][scope_key][0][record_key][0];
            match signal {
                Signal::Logs => assert_eq!(item["body"], original["body"], "{name}"),
                Signal::Metrics => {
                    assert_eq!(item["name"], original["name"], "{name}");
                    for kind in [
                        "sum",
                        "gauge",
                        "histogram",
                        "exponentialHistogram",
                        "summary",
                    ] {
                        if let Some(points) = original[kind]["dataPoints"].as_array() {
                            let actual = item[kind]["dataPoints"]
                                .as_array()
                                .expect("The metric data points must survive ingestion.");
                            assert_eq!(actual.len(), points.len(), "{name}");
                            for (actual, point) in actual.iter().zip(points) {
                                for key in ["asInt", "asDouble", "count", "sum", "timeUnixNano"] {
                                    if let Some(value) = point.get(key) {
                                        if matches!(key, "asInt" | "count") && value.is_number() {
                                            assert_eq!(
                                                actual[key].as_str().unwrap(),
                                                value.to_string(),
                                                "{name}: {key}"
                                            );
                                        } else if actual[key].is_number() && value.is_number() {
                                            assert_eq!(
                                                actual[key].as_f64(),
                                                value.as_f64(),
                                                "{name}: {key}"
                                            );
                                        } else {
                                            assert_eq!(&actual[key], value, "{name}: {key}");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Signal::Traces => {
                    for key in ["name", "startTimeUnixNano", "endTimeUnixNano"] {
                        assert_eq!(item[key], original[key], "{name}: {key}");
                    }
                    if name.ends_with(".json") {
                        assert_eq!(item["traceId"], original["traceId"], "{name}");
                        assert_eq!(item["spanId"], original["spanId"], "{name}");
                    }
                }
            }
        }
        if name == "codex-protobuf-traces.pb" {
            let span = &records[offset].payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
            assert_eq!(span["traceId"], "11111111111111111111111111111111");
            assert_eq!(span["spanId"], "f04bc934bd1fbc98");
            assert_eq!(span["parentSpanId"], "2222222222222222");
        }
        offset += count;
    }
    receiver.shutdown().await.unwrap();
    assert_eq!(log.read().unwrap().len(), 164);
}

#[tokio::test]
async fn startup_applies_configured_retention_and_publishes_the_bound_endpoint() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    std::fs::write(
        sandbox.path().join("config.toml"),
        "[telemetry]\nport = 0\nretention_days = 1\n",
    )
    .unwrap();
    let log = TelemetryLog::new(sandbox.path(), device.device_id);
    let payload = serde_json::json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": "old"}}]}]}]});
    log.append(
        Signal::Logs,
        vec![payload.clone()],
        super::now_ms() - 2 * 86_400_000,
    )
    .unwrap();
    let recent = log
        .append(Signal::Logs, vec![payload], super::now_ms())
        .unwrap();
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    assert_eq!(log.read().unwrap(), recent);
    let endpoint = sandbox.path().join("otlp.endpoint");
    assert_eq!(
        std::fs::read_to_string(&endpoint).unwrap().trim(),
        format!("http://{}", receiver.local_addr())
    );
    receiver.shutdown().await.unwrap();
    assert!(!endpoint.exists());
}

#[tokio::test]
async fn default_retention_is_thirty_days_and_maintenance_prunes_without_a_restart() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    let log = TelemetryLog::new(sandbox.path(), device.device_id);
    let payload = serde_json::json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": "record"}}]}]}]});
    log.append(
        Signal::Logs,
        vec![payload.clone()],
        super::now_ms() - 31 * 86_400_000,
    )
    .unwrap();
    let retained = log
        .append(
            Signal::Logs,
            vec![payload.clone()],
            super::now_ms() - 29 * 86_400_000,
        )
        .unwrap();
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    assert_eq!(log.read().unwrap(), retained);
    // An HTTP response proves that the server and its maintenance interval started.
    assert!(
        post(receiver.local_addr(), "/v1/logs", "application/json", b"{}")
            .await
            .starts_with("HTTP/1.1 200")
    );
    log.append(
        Signal::Logs,
        vec![payload],
        super::now_ms() - 31 * 86_400_000,
    )
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if log.read().unwrap() == retained {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Maintenance must remove expired records.");
    receiver.shutdown().await.unwrap();
}

#[tokio::test]
async fn uses_the_configured_port_and_reports_a_bind_conflict() {
    let sandbox = tempfile::tempdir().unwrap();
    Device::init(sandbox.path(), "test-device").unwrap();
    let occupied = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = occupied.local_addr().unwrap().port();
    std::fs::write(
        sandbox.path().join("config.toml"),
        format!("[telemetry]\nport = {port}\n"),
    )
    .unwrap();
    let error = match Receiver::bind(sandbox.path()).await {
        Err(error) => error,
        Ok(_) => panic!("The receiver must use the configured port."),
    };
    assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
    assert!(!sandbox.path().join("otlp.endpoint").exists());
}

#[tokio::test]
async fn rejects_invalid_requests_without_appending_records() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    for (path, content_type, body, status) in [
        ("/v1/logs", "application/json", b"{".as_slice(), "400"),
        (
            "/v1/metrics",
            "application/json",
            b"{\"resourceMetrics\":42}".as_slice(),
            "400",
        ),
        (
            "/v1/traces",
            "application/x-protobuf",
            b"\xff".as_slice(),
            "400",
        ),
        ("/v1/logs", "text/plain", b"{}".as_slice(), "415"),
    ] {
        let response = post(receiver.local_addr(), path, content_type, body).await;
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
    }
    assert!(
        TelemetryLog::new(sandbox.path(), device.device_id)
            .read()
            .unwrap()
            .is_empty()
    );
    receiver.shutdown().await.unwrap();
}

#[tokio::test]
async fn reports_a_maintenance_failure_to_the_daemon() {
    let sandbox = tempfile::tempdir().unwrap();
    Device::init(sandbox.path(), "test-device").unwrap();
    let mut receiver = Receiver::bind(sandbox.path()).await.unwrap();
    assert!(
        post(receiver.local_addr(), "/v1/logs", "application/json", b"{}")
            .await
            .starts_with("HTTP/1.1 200")
    );
    std::fs::write(
        sandbox.path().join("telemetry/log.jsonl"),
        b"invalid record\n",
    )
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    let error = tokio::time::timeout(Duration::from_secs(30), receiver.wait())
        .await
        .expect("The receiver must report its maintenance failure.")
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    receiver.shutdown().await.unwrap();
}

async fn post(address: SocketAddr, path: &str, content_type: &str, body: &[u8]) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let headers = format!("POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(headers.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    }).await.expect("The OTLP request exceeded the hang guard.")
}

#[tokio::test]
async fn redacts_codex_tool_content_and_email_before_storage_in_both_encodings() {
    for content in [false, true] {
        let sandbox = tempfile::tempdir().unwrap();
        let device = Device::init(sandbox.path(), "test-device").unwrap();
        if content {
            std::fs::write(
                sandbox.path().join("config.toml"),
                "[telemetry]\ncontent = true\n",
            )
            .unwrap();
        }
        let receiver = Receiver::bind(sandbox.path()).await.unwrap();
        let log = TelemetryLog::new(sandbox.path(), device.device_id);
        for (name, path, encoding) in [
            ("codex-baseline-logs.json", "/v1/logs", "application/json"),
            (
                "codex-protobuf-logs.pb",
                "/v1/logs",
                "application/x-protobuf",
            ),
            (
                "codex-baseline-metrics.json",
                "/v1/metrics",
                "application/json",
            ),
            (
                "codex-protobuf-metrics.pb",
                "/v1/metrics",
                "application/x-protobuf",
            ),
            (
                "codex-baseline-traces.json",
                "/v1/traces",
                "application/json",
            ),
            (
                "codex-protobuf-traces.pb",
                "/v1/traces",
                "application/x-protobuf",
            ),
        ] {
            let response = post(
                receiver.local_addr(),
                path,
                encoding,
                &std::fs::read(Path::new(FIXTURES).join(name)).unwrap(),
            )
            .await;
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        }
        let records = log.read().unwrap();
        let mut tools = 0;
        for record in &records {
            let serialized = serde_json::to_string(&record.payload).unwrap();
            assert!(!serialized.contains("user.email"));
            if record.signal != Signal::Logs {
                continue;
            }
            let item = &record.payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
            let attributes = item["attributes"].as_array().unwrap();
            if attributes.iter().any(|a| {
                a["key"] == "event.name" && a["value"]["stringValue"] == "codex.tool_result"
            }) {
                tools += 1;
                for key in ["arguments", "output"] {
                    assert_eq!(attributes.iter().any(|a| a["key"] == key), content, "{key}");
                }
                assert!(attributes.iter().any(|a| a["key"] == "tool_name"));
            }
        }
        assert_eq!(tools, 4);
        receiver.shutdown().await.unwrap();
        assert_eq!(log.read().unwrap(), records);
    }
}

#[tokio::test]
async fn native_boundaries_do_not_duplicate_canonical_turns_tools_or_models() {
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-device").unwrap();
    let receiver = Receiver::bind(sandbox.path()).await.unwrap();
    let payload = serde_json::json!({"resourceSpans": [{"scopeSpans": [
        {"scope":{"name":"codex_exec"}, "spans":[
            {"name":"run_turn"}, {"name":"session_task.turn"},
            {"name":"handle_tool_call_with_source"}, {"name":"dispatch_tool_call_with_state"},
            {"name":"exec_command"}, {"name":"unified_exec.exec_command"},
            {"name":"code_mode.broker.invoke_tool"}, {"name":"exec"},
            {"name":"ws.session.turn"}, {"name":"ws.session.tool"}, {"name":"ws.session.model_call"},
            {"name":"responses_websocket.stream_request"}
        ]},
        {"scope":{"name":"com.anthropic.claude_code.tracing"}, "spans":[
            {"name":"claude_code.tool"}, {"name":"claude_code.tool.execution"},
            {"name":"claude_code.llm_request"}, {"name":"claude_code.tool.blocked_on_user"}
        ]},
        {"scope":{"name":"other.instrumentation"}, "spans":[{"name":"exec"}]}
    ]}]});
    let response = post(
        receiver.local_addr(),
        "/v1/traces",
        "application/json",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let records = TelemetryLog::new(sandbox.path(), device.device_id)
        .read()
        .unwrap();
    let names: Vec<_> = records
        .iter()
        .map(|r| {
            r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(
        names,
        [
            "responses_websocket.stream_request",
            "claude_code.tool.blocked_on_user",
            "exec"
        ]
    );
    receiver.shutdown().await.unwrap();
}

#[tokio::test]
async fn late_model_logs_reach_the_shared_span_pipeline_without_a_restart() {
    use crate::sessions::{CreateSession, Sessions};
    use std::sync::{Arc, Mutex};

    let sandbox = tempfile::tempdir().unwrap();
    let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    sessions
        .append(
            session.id,
            "turn.started",
            serde_json::json!({"observed_at_us":1000}),
        )
        .unwrap();
    sessions
        .append(
            session.id,
            "turn.ended",
            serde_json::json!({"observed_at_us":9000}),
        )
        .unwrap();
    let shared = sessions.spans.clone();
    let store = Arc::new(Mutex::new(sessions));
    let receiver = Receiver::bind_with_spans(sandbox.path(), Some(shared))
        .await
        .unwrap();
    let body = serde_json::json!({"resourceLogs":[{"resource":{"attributes":[{"key":semconv::SESSION_ID,"value":{"stringValue":session.id.to_string()}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":"5000000","body":{"stringValue":"claude_code.api_request"},"attributes":[{"key":"duration_ms","value":{"intValue":"2"}},{"key":"output_tokens","value":{"intValue":"7"}}]}]}]}]});
    assert!(
        post(
            receiver.local_addr(),
            "/v1/logs",
            "application/json",
            &serde_json::to_vec(&body).unwrap()
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    let log = TelemetryLog::new(sandbox.path(), session.device_id);
    let models: Vec<_> = log
        .read()
        .unwrap()
        .into_iter()
        .filter(|r| {
            r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"]
                == semconv::MODEL_CALL_SPAN
        })
        .collect();
    assert_eq!(models.len(), 1);
    let span = &models[0].payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["startTimeUnixNano"], "3000000");
    assert_eq!(span["endTimeUnixNano"], "5000000");
    receiver.shutdown().await.unwrap();
    drop(store);
    let first = log.read().unwrap();
    drop(Sessions::open(sandbox.path(), 0).unwrap());
    assert_eq!(log.read().unwrap(), first);
}

#[tokio::test]
async fn native_tracing_waits_for_late_stream_timing_instead_of_duplicating_a_model() {
    use crate::sessions::{CreateSession, Sessions};
    use serde_json::json;

    let sandbox = tempfile::tempdir().unwrap();
    std::fs::write(
        sandbox.path().join("config.toml"),
        "[telemetry]\nnative_tracing = true\n",
    )
    .unwrap();
    let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    sessions
        .append(session.id, "turn.started", json!({"observed_at_us":1000}))
        .unwrap();
    sessions
        .append(session.id, "turn.ended", json!({"observed_at_us":9000}))
        .unwrap();
    let receiver = Receiver::bind_with_spans(sandbox.path(), Some(sessions.spans.clone()))
        .await
        .unwrap();
    let resource = json!({"attributes":[{"key":semconv::SESSION_ID,"value":{"stringValue":session.id.to_string()}}]});
    let logs = json!({"resourceLogs":[{"resource":resource,"scopeLogs":[{"logRecords":[{"timeUnixNano":"5000000","attributes":[{"key":"event.name","value":{"stringValue":"codex.sse_event"}},{"key":"event.kind","value":{"stringValue":"response.completed"}},{"key":"output_token_count","value":{"stringValue":"7"}}]}]}]}]});
    let traces = json!({"resourceSpans":[{"resource":resource,"scopeSpans":[{"scope":{"name":"codex_exec"},"spans":[{"name":"responses_websocket.stream_request","startTimeUnixNano":"3000000","endTimeUnixNano":"5000000","traceId":"11111111111111111111111111111111","spanId":"2222222222222222"}]}]}]});
    assert!(
        post(
            receiver.local_addr(),
            "/v1/logs",
            "application/json",
            &serde_json::to_vec(&logs).unwrap()
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    let log = TelemetryLog::new(sandbox.path(), session.device_id);
    let count_models = || {
        log.read()
            .unwrap()
            .iter()
            .filter(|r| {
                r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"]
                    == semconv::MODEL_CALL_SPAN
            })
            .count()
    };
    assert_eq!(count_models(), 0);
    assert!(
        post(
            receiver.local_addr(),
            "/v1/traces",
            "application/json",
            &serde_json::to_vec(&traces).unwrap()
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    assert_eq!(count_models(), 1);
    let records = log.read().unwrap();
    let model = records
        .iter()
        .find(|r| {
            r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"]
                == semconv::MODEL_CALL_SPAN
        })
        .unwrap();
    let model = &model.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(model["startTimeUnixNano"], "3000000");
    assert!(
        model["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["key"] == semconv::TIMING && a["value"]["stringValue"] == "measured")
    );
    receiver.shutdown().await.unwrap();
}
