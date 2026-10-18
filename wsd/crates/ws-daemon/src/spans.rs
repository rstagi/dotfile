//! Canonical turn and tool spans from durable hook events.

use std::{
    collections::{HashMap, HashSet},
    io,
    path::Path,
};

use serde_json::{Value, json};
use ws_core::{
    ids::{DeviceId, SessionId},
    log::LogEntry,
    semconv,
    telemetry_log::{Signal, TelemetryLog},
};

pub(crate) struct Spans {
    log: TelemetryLog,
    written: HashSet<(String, String)>,
    sessions: HashMap<SessionId, SessionSpans>,
}

#[derive(Clone, Default)]
struct SessionSpans {
    resource: Vec<Value>,
    turn: Option<Turn>,
    tool_results: HashMap<String, u64>,
}

#[derive(Clone)]
struct Turn {
    source_id: Option<String>,
    span: Value,
    tools: HashMap<String, Value>,
    seen_tools: HashSet<String>,
}

impl Spans {
    pub(crate) fn open(state: &Path, device: DeviceId) -> io::Result<Self> {
        let log = TelemetryLog::new(state, device);
        let mut written = HashSet::new();
        for record in log.read()? {
            if record.signal != Signal::Traces {
                continue;
            }
            let span = &record.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
            if matches!(
                span["name"].as_str(),
                Some(semconv::TURN_SPAN | semconv::TOOL_SPAN)
            ) {
                written.insert((
                    span["traceId"].as_str().unwrap_or("").into(),
                    span["spanId"].as_str().unwrap_or("").into(),
                ));
            }
        }
        Ok(Self {
            log,
            written,
            sessions: HashMap::new(),
        })
    }

    /// Consumes each durable entry once. Replay uses the same source-derived span ids.
    pub(crate) fn observe(&mut self, entry: &LogEntry) -> io::Result<()> {
        let event = entry.event_type.as_str();
        if event == "hook.received" {
            if matches!(
                entry.body["payload"]["hook_event_name"].as_str(),
                Some(
                    "PostToolUse"
                        | "PostToolUseFailure"
                        | "post_tool_use"
                        | "post_tool_use_failure"
                )
            ) && let Some(callback) = entry.body["callback_id"].as_str()
            {
                self.sessions
                    .entry(entry.session_id)
                    .or_default()
                    .tool_results
                    .insert(callback.into(), tool_status(&entry.body["payload"]));
            }
            return Ok(());
        }
        if !matches!(
            event,
            "session.created"
                | "turn.started"
                | "turn.ended"
                | "turn.continued"
                | "turn.cancelled"
                | "tool.pre"
                | "tool.post"
                | "exit"
                | "process.exited"
                | "session.failed"
                | "session.finished"
                | "session.orphaned"
                | "done"
        ) {
            return Ok(());
        }
        if entry.body["state_accepted"] == false
            && matches!(
                event,
                "turn.started"
                    | "turn.ended"
                    | "turn.continued"
                    | "turn.cancelled"
                    | "tool.pre"
                    | "tool.post"
            )
        {
            return Ok(());
        }
        // Commit memory only after the telemetry append succeeds. Session-log replay repairs a failed append.
        let mut state = self
            .sessions
            .get(&entry.session_id)
            .cloned()
            .unwrap_or_default();
        let result = entry.body["callback_id"]
            .as_str()
            .and_then(|id| state.tool_results.remove(id));
        let mut finished = Vec::new();
        if event == "session.created" {
            state.resource = vec![attribute(semconv::SESSION_ID, entry.session_id.to_string())];
            for (key, source) in [
                (semconv::PARENT_SESSION_ID, "parent_session_id"),
                (semconv::PROJECT, "project"),
            ] {
                if let Some(value) = entry.body[source].as_str() {
                    state.resource.push(attribute(key, value));
                }
            }
        } else if event == "turn.started" {
            close_turn(&mut state, entry, true, &mut finished);
            let id = turn_id(entry)
                .map(str::to_owned)
                .unwrap_or_else(|| entry.event_id.to_string());
            let mut span = new_span(entry, semconv::TURN_SPAN);
            span["attributes"]
                .as_array_mut()
                .unwrap()
                .push(attribute(semconv::TURN_ID, &id));
            state.turn = Some(Turn {
                source_id: turn_id(entry).map(str::to_owned),
                span,
                tools: HashMap::new(),
                seen_tools: HashSet::new(),
            });
        } else if let Some(turn) = state.turn.as_mut() {
            let same_turn = turn
                .source_id
                .as_deref()
                .zip(turn_id(entry))
                .is_none_or(|(current, incoming)| current == incoming);
            match event {
                "tool.pre" if same_turn => {
                    if let Some(id) = entry.body["tool_use_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        && turn.seen_tools.insert(id.into())
                    {
                        let mut span = new_span(entry, semconv::TOOL_SPAN);
                        span["parentSpanId"] = turn.span["spanId"].clone();
                        let attributes = span["attributes"].as_array_mut().unwrap();
                        attributes.push(attribute(semconv::TOOL_USE_ID, id));
                        if let Some(name) = entry.body["tool_name"].as_str() {
                            attributes.push(attribute(semconv::TOOL_NAME, name));
                        }
                        turn.tools.insert(id.into(), span);
                    }
                }
                "tool.post" if same_turn => {
                    if let Some(id) = entry.body["tool_use_id"].as_str()
                        && let Some(mut span) = turn.tools.remove(id)
                    {
                        finish(
                            &mut span,
                            entry,
                            entry.body["tool_status"].as_u64().or(result).unwrap_or(0),
                            semconv::TOOL_POST,
                        );
                        finished.push(span);
                    }
                }
                "turn.ended" if same_turn && entry.body["stop_reply"]["decision"] != "block" => {
                    close_turn(&mut state, entry, false, &mut finished)
                }
                "turn.cancelled" if same_turn => close_turn(&mut state, entry, true, &mut finished),
                "exit" | "process.exited" | "session.failed" | "session.finished"
                | "session.orphaned" | "done" => close_turn(&mut state, entry, true, &mut finished),
                _ => {}
            }
        }
        let pending: Vec<_> = finished
            .into_iter()
            .filter(|span| !self.written.contains(&identity(span)))
            .collect();
        if !pending.is_empty() {
            let payloads = pending.iter().map(|span| json!({"resourceSpans": [{"resource": {"attributes": state.resource}, "scopeSpans": [{"scope": {"name": "ws.spans", "version": semconv::VERSION}, "spans": [span]}]}]})).collect();
            self.log.append(Signal::Traces, payloads, entry.ts_ms)?;
            self.written.extend(pending.iter().map(identity));
        }
        self.sessions.insert(entry.session_id, state);
        Ok(())
    }
}

/// Returns only the status that the hook result supports. Empty Codex results prove no command outcome.
pub(crate) fn tool_status(payload: &Value) -> u64 {
    let response = &payload["tool_response"];
    let failed = matches!(
        payload["hook_event_name"].as_str(),
        Some("PostToolUseFailure" | "post_tool_use_failure")
    ) || payload["error"].as_str().is_some_and(|s| !s.is_empty())
        || [payload, response].iter().any(|v| {
            v["is_error"] == true
                || v["isError"] == true
                || v.get("error").is_some_and(|error| match error {
                    Value::Null | Value::Bool(false) => false,
                    Value::String(message) => !message.is_empty(),
                    _ => true,
                })
                || v["interrupted"] == true
                || v["success"] == false
                || ["exit_code", "exitCode"]
                    .iter()
                    .any(|key| v[*key].as_i64().is_some_and(|code| code != 0))
        });
    if failed {
        2
    } else if response.is_object() || response.as_str().is_some_and(|s| !s.is_empty()) {
        1
    } else {
        0
    }
}

// S2 producer boundaries overlap the hook spans. Keep native model and transport timing.
pub(crate) fn is_native_boundary(payload: &Value) -> bool {
    let scope = &payload["resourceSpans"][0]["scopeSpans"][0];
    let name = scope["spans"][0]["name"].as_str().unwrap_or("");
    if matches!(name, semconv::TURN_SPAN | semconv::TOOL_SPAN) {
        return true;
    }
    match scope["scope"]["name"].as_str() {
        Some("com.anthropic.claude_code.tracing") => {
            matches!(name, "claude_code.tool" | "claude_code.tool.execution")
        }
        Some("codex_exec") => matches!(
            name,
            "run_turn"
                | "session_task.turn"
                | "handle_tool_call_with_source"
                | "dispatch_tool_call_with_state"
                | "exec_command"
                | "unified_exec.exec_command"
                | "code_mode.broker.invoke_tool"
                | "exec"
        ),
        _ => false,
    }
}

fn close_turn(state: &mut SessionSpans, entry: &LogEntry, error: bool, finished: &mut Vec<Value>) {
    let Some(mut turn) = state.turn.take() else {
        return;
    };
    state.tool_results.clear();
    for (_, mut tool) in turn.tools.drain() {
        finish(&mut tool, entry, 2, closure_name(entry));
        finished.push(tool);
    }
    finish(
        &mut turn.span,
        entry,
        if error { 2 } else { 1 },
        closure_name(entry),
    );
    finished.push(turn.span);
}

fn new_span(entry: &LogEntry, name: &str) -> Value {
    json!({"traceId": format!("{:032x}", u128::from(entry.session_id)),
        "spanId": format!("{:016x}", (u128::from(entry.event_id) as u64).max(1)),
        "name": name, "kind": 1, "startTimeUnixNano": timestamp(entry).to_string(),
        "attributes": [attribute(semconv::SEMCONV_VERSION, semconv::VERSION), attribute(semconv::EVENT_ID, entry.event_id.to_string())],
        "events": [span_event(entry, if name == semconv::TURN_SPAN { semconv::TURN_STARTED } else { semconv::TOOL_PRE })]})
}

fn turn_id(entry: &LogEntry) -> Option<&str> {
    ["prompt_id", "turn_id"]
        .iter()
        .find_map(|key| entry.body[*key].as_str().filter(|id| !id.is_empty()))
}

fn finish(span: &mut Value, entry: &LogEntry, code: u64, event: &str) {
    let start = span["startTimeUnixNano"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    span["endTimeUnixNano"] = json!(timestamp(entry).max(start).to_string());
    span["status"] = json!({"code": code});
    if code == 2 {
        span["attributes"]
            .as_array_mut()
            .unwrap()
            .push(attribute(semconv::REASON, &entry.event_type));
    }
    span["events"]
        .as_array_mut()
        .unwrap()
        .push(span_event(entry, event));
}

fn closure_name(entry: &LogEntry) -> &'static str {
    match entry.event_type.as_str() {
        "turn.ended" => semconv::TURN_ENDED,
        "turn.cancelled" => semconv::TURN_CANCELLED,
        "session.failed" => semconv::SESSION_FAILED,
        "session.finished" | "done" => semconv::SESSION_FINISHED,
        "session.orphaned" => semconv::SESSION_ORPHANED,
        "turn.started" => semconv::TURN_STARTED,
        _ => semconv::SESSION_EXIT,
    }
}

fn span_event(entry: &LogEntry, name: &str) -> Value {
    json!({"name": name, "timeUnixNano": timestamp(entry).to_string(), "attributes": [attribute(semconv::EVENT_ID, entry.event_id.to_string()), attribute(semconv::SEMCONV_VERSION, semconv::VERSION), attribute(semconv::EVENT_DOMAIN, semconv::SESSION_DOMAIN)]})
}

fn timestamp(entry: &LogEntry) -> u64 {
    entry.body["observed_at_us"]
        .as_u64()
        .map(|us| us.saturating_mul(1000))
        .unwrap_or_else(|| entry.ts_ms.saturating_mul(1_000_000))
}

fn attribute(key: &str, value: impl Into<String>) -> Value {
    json!({"key": key, "value": {"stringValue": value.into()}})
}

fn identity(span: &Value) -> (String, String) {
    (
        span["traceId"].as_str().unwrap().into(),
        span["spanId"].as_str().unwrap().into(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use ws_core::{
        ids::Ulid,
        semconv,
        telemetry_log::{Signal, TelemetryLog},
    };

    use crate::{
        hooks::{self, Callback, StopDecision},
        sessions::{CreateSession, Sessions},
    };

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/spikes/s2/");

    #[tokio::test]
    async fn s2_shell_hooks_write_one_turn_and_one_child_tool() {
        for (name, expected_status) in [
            ("baseline-hooks.jsonl", 1),
            ("detailed-json-hooks.jsonl", 1),
            ("codex-baseline-hooks.jsonl", 1),
            ("codex-trace-off-hooks.jsonl", 1),
            ("codex-failure-hooks.jsonl", 0),
        ] {
            let sandbox = tempfile::tempdir().unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let store = Arc::new(Mutex::new(sessions));
            if expected_status == 0 {
                let native: Value = serde_json::from_str(
                    &std::fs::read_to_string(format!("{FIXTURES}codex-failure-logs.json")).unwrap(),
                )
                .unwrap();
                // This captured failure reports native success=true. The hook contains no exit status.
                log.append(Signal::Logs, vec![native], 0).unwrap();
            }
            let samples = std::fs::read_to_string(format!("{FIXTURES}{name}")).unwrap();
            for line in samples.lines() {
                let sample: Value = serde_json::from_str(line).unwrap();
                let payload = sample["payload"].clone();
                let state = match payload["hook_event_name"].as_str().unwrap() {
                    "SessionStart" => continue,
                    "SessionEnd" => "exit",
                    "Stop" => "stop",
                    _ => "working",
                };
                hooks::ingest(
                    store.clone(),
                    Callback {
                        callback_id: Ulid::new(),
                        session_id: session.id,
                        observed_at_us: sample["received_unix_ns"].as_u64().unwrap() / 1000,
                        state: state.into(),
                        pane: None,
                        tmux_socket: None,
                        tmux_path: None,
                        payload,
                        notify: None,
                    },
                    StopDecision::Accept,
                )
                .await
                .unwrap();
            }
            let records: Vec<_> = log
                .read()
                .unwrap()
                .into_iter()
                .filter(|r| r.signal == Signal::Traces)
                .collect();
            assert_eq!(records.len(), 2, "{name}");
            let spans: Vec<_> = records
                .iter()
                .map(|r| &r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0])
                .collect();
            let turn = spans
                .iter()
                .find(|s| s["name"] == semconv::TURN_SPAN)
                .unwrap();
            let tool = spans
                .iter()
                .find(|s| s["name"] == semconv::TOOL_SPAN)
                .unwrap();
            assert_eq!(turn["traceId"], format!("{:032x}", u128::from(session.id)));
            assert_eq!(tool["traceId"], turn["traceId"]);
            assert_eq!(tool["parentSpanId"], turn["spanId"]);
            assert_eq!(turn["status"]["code"], 1);
            assert_eq!(tool["status"]["code"], expected_status);
            assert_eq!(attribute(tool, semconv::TOOL_NAME), json!("Bash"));
            assert_eq!(attribute(turn, semconv::SEMCONV_VERSION), json!("1"));
            // The persisted request must decode with the same OTLP schema as received spans.
            for record in records {
                serde_json::from_value::<
                    opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest,
                >(record.payload)
                .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn interruption_without_a_prompt_id_closes_the_turn_and_tool() {
        for cancellation in [
            json!({"hook_event_name":"Interrupt"}),
            json!({"hook_event_name":"Interrupt", "turn_id":"known-later"}),
        ] {
            let sandbox = tempfile::tempdir().unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let store = Arc::new(Mutex::new(sessions));
            hook(
                &store,
                session.id,
                100,
                "working",
                json!({"hook_event_name":"UserPromptSubmit"}),
            )
            .await;
            hook(
                &store,
                session.id,
                200,
                "working",
                json!({"hook_event_name":"PreToolUse", "tool_use_id":"lost", "tool_name":"Bash"}),
            )
            .await;
            hook(
                &store,
                session.id,
                250,
                "waiting",
                json!({"hook_event_name":"PermissionRequest"}),
            )
            .await;
            hook(&store, session.id, 300, "cancel", cancellation).await;
            let spans = stored_spans(&log);
            assert_eq!(spans.len(), 2);
            for span in spans {
                assert_eq!(span["status"]["code"], 2);
                assert_eq!(span["endTimeUnixNano"], "300000");
                assert_eq!(
                    span["events"].as_array().unwrap().last().unwrap()["name"],
                    semconv::TURN_CANCELLED
                );
            }
        }
    }

    #[test]
    fn replay_uses_legacy_hook_results_and_retains_span_ids() {
        use ws_core::log::SessionLog;
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        drop(sessions);
        let source = SessionLog::new(sandbox.path(), session.id, session.device_id);
        source
            .append(
                0,
                "turn.started",
                json!({"observed_at_us":100, "prompt_id":"turn"}),
            )
            .unwrap();
        source
            .append(
                0,
                "tool.pre",
                json!({"observed_at_us":200, "tool_use_id":"tool", "prompt_id":"turn"}),
            )
            .unwrap();
        let callback = Ulid::new();
        source.append(0, "hook.received", json!({"callback_id":callback, "payload":{"hook_event_name":"PostToolUse", "tool_response":{"exit_code":7}}})).unwrap();
        source.append(0, "tool.post", json!({"callback_id":callback, "observed_at_us":300, "tool_use_id":"tool", "prompt_id":"turn"})).unwrap();
        source
            .append(
                0,
                "turn.ended",
                json!({"observed_at_us":400, "prompt_id":"turn"}),
            )
            .unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let first = log.read().unwrap();
        assert_eq!(first.len(), 2);
        let spans = stored_spans(&log);
        let tool = spans
            .iter()
            .find(|s| s["name"] == semconv::TOOL_SPAN)
            .unwrap();
        assert_eq!(tool["status"]["code"], 2);
        drop(sessions);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        assert_eq!(log.read().unwrap(), first);
    }

    #[tokio::test]
    async fn a_missing_start_id_does_not_reject_later_hook_ids() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let store = Arc::new(Mutex::new(sessions));
        hook(
            &store,
            session.id,
            100,
            "working",
            json!({"hook_event_name":"UserPromptSubmit"}),
        )
        .await;
        hook(
            &store,
            session.id,
            200,
            "working",
            json!({"hook_event_name":"PreToolUse", "tool_use_id":"tool", "turn_id":"known-later"}),
        )
        .await;
        hook(&store, session.id, 300, "working", json!({"hook_event_name":"PostToolUse", "tool_use_id":"tool", "turn_id":"known-later", "tool_response":"ok"})).await;
        hook(
            &store,
            session.id,
            400,
            "stop",
            json!({"hook_event_name":"Stop", "turn_id":"known-later"}),
        )
        .await;
        let spans = stored_spans(&log);
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|span| span["status"]["code"] == 1));
        let source = ws_core::log::SessionLog::new(sandbox.path(), session.id, session.device_id)
            .read()
            .unwrap();
        let start = source
            .entries
            .iter()
            .find(|e| e.event_type == "turn.started")
            .unwrap();
        let turn = spans
            .iter()
            .find(|span| span["name"] == semconv::TURN_SPAN)
            .unwrap();
        assert_eq!(
            attribute(turn, semconv::TURN_ID),
            start.event_id.to_string()
        );
    }

    #[tokio::test]
    async fn tool_status_uses_explicit_hook_failures_and_keeps_unknown_results_unset() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let store = Arc::new(Mutex::new(sessions));
        hook(
            &store,
            session.id,
            100,
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"turn"}),
        )
        .await;
        let cases = [
            (
                json!({"hook_event_name":"PostToolUseFailure", "error":"failed"}),
                2,
            ),
            (json!({"tool_response":{"exit_code":7}}), 2),
            (json!({"tool_response":{"interrupted":true}}), 2),
            (json!({"tool_response":{"is_error":true}}), 2),
            (json!({"tool_response":{"isError":true}}), 2),
            (json!({"tool_response":{"error":{"message":"failed"}}}), 2),
            (json!({"tool_response":{"exit_code":0,"stdout":"ok"}}), 1),
            (json!({"tool_response":"", "success":true}), 0),
        ];
        for (index, (mut payload, expected)) in cases.into_iter().enumerate() {
            let id = format!("tool-{index}");
            let time = 200 + index as u64 * 100;
            hook(
                &store,
                session.id,
                time,
                "working",
                json!({"hook_event_name":"PreToolUse", "tool_use_id":id}),
            )
            .await;
            if payload.get("hook_event_name").is_none() {
                payload["hook_event_name"] = json!("PostToolUse");
            }
            payload["tool_use_id"] = json!(id);
            hook(&store, session.id, time + 10, "working", payload).await;
            let spans = stored_spans(&log);
            assert_eq!(spans.last().unwrap()["status"]["code"], expected, "{id}");
        }
    }

    #[test]
    fn exits_and_abandonment_close_every_open_span() {
        use ws_core::log::SessionLog;
        for event in [
            "exit",
            "process.exited",
            "session.failed",
            "session.finished",
            "session.orphaned",
            "done",
        ] {
            let sandbox = tempfile::tempdir().unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            drop(sessions);
            let source = SessionLog::new(sandbox.path(), session.id, session.device_id);
            source
                .append(
                    0,
                    "turn.started",
                    json!({"observed_at_us":100, "prompt_id":"turn"}),
                )
                .unwrap();
            source
                .append(
                    0,
                    "tool.pre",
                    json!({"observed_at_us":200, "tool_use_id":"lost"}),
                )
                .unwrap();
            source
                .append(
                    0,
                    event,
                    json!({"observed_at_us":300, "reason":"abandoned", "code":0}),
                )
                .unwrap();
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let spans = stored_spans(&log);
            assert_eq!(spans.len(), 2, "{event}");
            for span in spans {
                assert_eq!(span["status"]["code"], 2, "{event}");
                assert_eq!(span["endTimeUnixNano"], "300000", "{event}");
            }
        }
    }

    #[test]
    fn tools_match_by_id_within_a_session_and_turn() {
        use ws_core::log::SessionLog;
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let a = sessions.create(CreateSession::default()).unwrap();
        let b = sessions.create(CreateSession::default()).unwrap();
        drop(sessions);
        for session in [&a, &b] {
            let source = SessionLog::new(sandbox.path(), session.id, session.device_id);
            for (event, body) in [
                (
                    "turn.started",
                    json!({"observed_at_us":100, "turn_id":"one"}),
                ),
                (
                    "tool.pre",
                    json!({"observed_at_us":200, "tool_use_id":"first"}),
                ),
                (
                    "tool.pre",
                    json!({"observed_at_us":210, "tool_use_id":"second"}),
                ),
                ("turn.continued", json!({"observed_at_us":220})),
                (
                    "tool.post",
                    json!({"observed_at_us":250, "tool_use_id":"second", "tool_status":1}),
                ),
                (
                    "tool.post",
                    json!({"observed_at_us":260, "tool_use_id":"first", "tool_status":2}),
                ),
                (
                    "tool.pre",
                    json!({"observed_at_us":270, "tool_use_id":"missing"}),
                ),
                (
                    "turn.started",
                    json!({"observed_at_us":300, "turn_id":"two"}),
                ),
                (
                    "tool.pre",
                    json!({"observed_at_us":350, "tool_use_id":"first", "turn_id":"two"}),
                ),
                (
                    "tool.post",
                    json!({"observed_at_us":360, "tool_use_id":"first", "turn_id":"one", "tool_status":1}),
                ),
                (
                    "tool.post",
                    json!({"observed_at_us":370, "tool_use_id":"first", "turn_id":"two", "tool_status":1}),
                ),
                ("turn.ended", json!({"observed_at_us":400, "turn_id":"two"})),
                (
                    "tool.post",
                    json!({"observed_at_us":500, "tool_use_id":"missing", "tool_status":1}),
                ),
            ] {
                source.append(0, event, body).unwrap();
            }
        }
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        let spans = stored_spans(&TelemetryLog::new(sandbox.path(), a.device_id));
        assert_eq!(spans.len(), 12);
        for session in [&a, &b] {
            let trace = format!("{:032x}", u128::from(session.id));
            let turns: Vec<_> = spans
                .iter()
                .filter(|s| s["traceId"] == trace && s["name"] == semconv::TURN_SPAN)
                .collect();
            assert_eq!(turns.len(), 2);
            let one = turns
                .iter()
                .find(|s| attribute(s, semconv::TURN_ID) == "one")
                .unwrap();
            let two = turns
                .iter()
                .find(|s| attribute(s, semconv::TURN_ID) == "two")
                .unwrap();
            assert_eq!(one["status"]["code"], 2);
            assert_eq!(one["endTimeUnixNano"], "300000");
            assert_eq!(two["status"]["code"], 1);
            let tools: Vec<_> = spans
                .iter()
                .filter(|s| s["traceId"] == trace && s["name"] == semconv::TOOL_SPAN)
                .collect();
            let first_turn_tool = tools
                .iter()
                .find(|s| {
                    s["parentSpanId"] == one["spanId"]
                        && attribute(s, semconv::TOOL_USE_ID) == "first"
                })
                .unwrap();
            assert_eq!(first_turn_tool["endTimeUnixNano"], "260000");
            assert_eq!(first_turn_tool["status"]["code"], 2);
            let second_turn_tool = tools
                .iter()
                .find(|s| s["parentSpanId"] == two["spanId"])
                .unwrap();
            assert_eq!(second_turn_tool["endTimeUnixNano"], "370000");
            assert_eq!(second_turn_tool["status"]["code"], 1);
            let missing = tools
                .iter()
                .find(|s| attribute(s, semconv::TOOL_USE_ID) == "missing")
                .unwrap();
            assert_eq!(missing["status"]["code"], 2);
            assert_eq!(missing["endTimeUnixNano"], "300000");
        }
    }

    #[tokio::test]
    async fn a_blocked_stop_keeps_the_turn_open_and_an_accepted_stop_closes_missing_posts() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let store = Arc::new(Mutex::new(sessions));
        hook(
            &store,
            session.id,
            100,
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"turn"}),
        )
        .await;
        hook(
            &store,
            session.id,
            200,
            "working",
            json!({"hook_event_name":"PreToolUse", "tool_use_id":"lost"}),
        )
        .await;
        hooks::ingest(
            store.clone(),
            Callback {
                callback_id: Ulid::new(),
                session_id: session.id,
                observed_at_us: 300,
                state: "stop".into(),
                pane: None,
                tmux_socket: None,
                tmux_path: None,
                payload: json!({"hook_event_name":"Stop", "prompt_id":"turn"}),
                notify: None,
            },
            StopDecision::Block("Continue the turn.".into()),
        )
        .await
        .unwrap();
        assert!(log.read().unwrap().is_empty());
        hook(
            &store,
            session.id,
            400,
            "stop",
            json!({"hook_event_name":"Stop", "prompt_id":"turn"}),
        )
        .await;
        let spans = stored_spans(&log);
        assert_eq!(spans.len(), 2);
        let turn = spans
            .iter()
            .find(|s| s["name"] == semconv::TURN_SPAN)
            .unwrap();
        let tool = spans
            .iter()
            .find(|s| s["name"] == semconv::TOOL_SPAN)
            .unwrap();
        assert_eq!(turn["status"]["code"], 1);
        assert_eq!(tool["status"]["code"], 2);
        assert_eq!(tool["endTimeUnixNano"], "400000");
    }

    #[tokio::test]
    async fn a_failed_telemetry_append_recovers_on_callback_retry_and_restart() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let store = Arc::new(Mutex::new(sessions));
        hook(
            &store,
            session.id,
            100,
            "working",
            json!({"hook_event_name":"UserPromptSubmit", "prompt_id":"turn"}),
        )
        .await;
        hook(
            &store,
            session.id,
            200,
            "working",
            json!({"hook_event_name":"PreToolUse", "tool_use_id":"tool"}),
        )
        .await;
        let callback = Callback {
            callback_id: Ulid::new(),
            session_id: session.id,
            observed_at_us: 300,
            state: "working".into(),
            pane: None,
            tmux_socket: None,
            tmux_path: None,
            payload: json!({"hook_event_name":"PostToolUse", "tool_use_id":"tool", "tool_response":"ok"}),
            notify: None,
        };
        let broken = sandbox.path().join("telemetry/log.jsonl");
        std::fs::create_dir(&broken).unwrap();
        assert!(
            hooks::ingest(store.clone(), callback.clone(), StopDecision::Accept)
                .await
                .is_err()
        );
        std::fs::remove_dir(&broken).unwrap();
        hooks::ingest(store.clone(), callback, StopDecision::Accept)
            .await
            .unwrap();
        assert_eq!(stored_spans(&log).len(), 1);
        drop(store);
        let sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let store = Arc::new(Mutex::new(sessions));
        hook(
            &store,
            session.id,
            400,
            "stop",
            json!({"hook_event_name":"Stop", "prompt_id":"turn"}),
        )
        .await;
        let first = log.read().unwrap();
        assert_eq!(first.len(), 2);
        drop(store);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        assert_eq!(log.read().unwrap(), first);
    }

    #[test]
    fn the_trace_id_decodes_the_session_ulid_and_resources_retain_session_metadata() {
        use ws_core::{device::Device, log::SessionLog};
        let sandbox = tempfile::tempdir().unwrap();
        let device = Device::init(sandbox.path(), "test-device").unwrap();
        let session = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let source = SessionLog::new(sandbox.path(), session, device.device_id);
        source.append(0, "session.created", json!({"alias":1,"parent_session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAA","project":"example"})).unwrap();
        source
            .append(
                0,
                "turn.started",
                json!({"observed_at_us":100,"prompt_id":"","turn_id":"turn"}),
            )
            .unwrap();
        source
            .append(
                0,
                "turn.ended",
                json!({"observed_at_us":200,"prompt_id":"turn"}),
            )
            .unwrap();
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        let log = TelemetryLog::new(sandbox.path(), device.device_id);
        let spans = stored_spans(&log);
        assert_eq!(spans[0]["traceId"], "01563e3ab5d3d6764c61efb99302bd5b");
        assert_eq!(attribute(&spans[0], semconv::TURN_ID), "turn");
        assert_eq!(spans[0]["startTimeUnixNano"], "100000");
        assert_eq!(spans[0]["endTimeUnixNano"], "200000");
        let records = log.read().unwrap();
        let resource = &records[0].payload["resourceSpans"][0]["resource"];
        assert_eq!(
            attribute(resource, semconv::SESSION_ID),
            "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        );
        assert_eq!(
            attribute(resource, semconv::PARENT_SESSION_ID),
            "01ARZ3NDEKTSV4RRFFQ69G5FAA"
        );
        assert_eq!(attribute(resource, semconv::PROJECT), "example");
        assert_eq!(
            attribute(resource, semconv::DEVICE_ID),
            device.device_id.to_string()
        );
    }

    async fn hook(
        store: &Arc<Mutex<Sessions>>,
        session: Ulid,
        time: u64,
        state: &str,
        payload: Value,
    ) {
        hooks::ingest(
            store.clone(),
            Callback {
                callback_id: Ulid::new(),
                session_id: session,
                observed_at_us: time,
                state: state.into(),
                pane: None,
                tmux_socket: None,
                tmux_path: None,
                payload,
                notify: None,
            },
            StopDecision::Accept,
        )
        .await
        .unwrap();
    }

    fn stored_spans(log: &TelemetryLog) -> Vec<Value> {
        log.read()
            .unwrap()
            .iter()
            .filter(|r| r.signal == Signal::Traces)
            .map(|r| r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0].clone())
            .collect()
    }

    fn attribute(span: &Value, name: &str) -> Value {
        span["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["key"] == name)
            .unwrap()["value"]["stringValue"]
            .clone()
    }
}
