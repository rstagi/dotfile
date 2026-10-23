//! OTLP log records from durable session events.

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use ws_core::{
    ids::{EventId, SessionId},
    log::LogEntry,
    semconv,
    telemetry_log::{Signal, TelemetryLog, TelemetryRecord},
};

pub(crate) struct Emitter {
    state: PathBuf,
    content: bool,
    written: HashSet<EventId>,
    resources: HashMap<SessionId, Vec<Value>>,
    phases: HashMap<SessionId, Value>,
}

impl Emitter {
    /// Opens an emitter without a record of written events. The owner passes each stored record to [`Self::note_written`].
    pub(crate) fn open(state: &Path) -> io::Result<Self> {
        Ok(Self {
            state: state.into(),
            content: ws_core::config::Config::load(state)?.telemetry.content,
            written: HashSet::new(),
            resources: HashMap::new(),
            phases: HashMap::new(),
        })
    }

    /// Records the session events that a stored telemetry record holds. Replay skips them.
    pub(crate) fn note_written(&mut self, record: &TelemetryRecord) {
        if record.signal != Signal::Logs {
            return;
        }
        for resource in record.payload["resourceLogs"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for scope in resource["scopeLogs"].as_array().into_iter().flatten() {
                if scope["scope"]["name"] != semconv::SESSION_DOMAIN {
                    continue;
                }
                for record in scope["logRecords"].as_array().into_iter().flatten() {
                    if let Some(id) = record["attributes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|a| a["key"] == semconv::EVENT_ID)
                        .and_then(|a| a["value"]["stringValue"].as_str())
                        .and_then(|id| id.parse().ok())
                    {
                        self.written.insert(id);
                    }
                }
            }
        }
    }

    pub(crate) fn observe(&mut self, entry: &LogEntry) -> io::Result<Option<Value>> {
        if entry.event_type == "session.created" {
            let mut resource = vec![attribute(semconv::SESSION_ID, json!(entry.session_id))];
            if let Some(parent) = entry.body["parent_session_id"].as_str() {
                resource.push(attribute(semconv::PARENT_SESSION_ID, json!(parent)));
            }
            if let Some(project) = session_project(&entry.body) {
                resource.push(attribute(semconv::PROJECT, json!(project)));
            }
            self.resources.insert(entry.session_id, resource);
            if entry.body.get("phase").is_some() {
                self.phases.insert(entry.session_id, entry.body.clone());
            }
        }
        let phase = self.phases.get(&entry.session_id);
        let Some(name) = mapped_name(entry, phase.is_some()) else {
            return Ok(None);
        };
        let attributes = event_attributes(entry, name, phase);
        let event = json!({"name": name, "timeUnixNano": timestamp(entry).to_string(), "attributes": attributes});
        if self.written.contains(&entry.event_id) {
            return Ok(Some(event));
        }
        let mut body = entry.body.clone();
        redact_body(&mut body, None, self.content);
        let record = json!({"timeUnixNano": timestamp(entry).to_string(),
            "traceId": format!("{:032x}", u128::from(entry.session_id)),
            "eventName": name, "attributes": attributes,
            "body": {"kvlistValue": {"values": [
                {"key": semconv::EVENT_NAME, "value": {"stringValue": name}},
                {"key": "body", "value": any_value(&body)}]}}});
        let resource = self
            .resources
            .get(&entry.session_id)
            .cloned()
            .unwrap_or_else(|| vec![attribute(semconv::SESSION_ID, json!(entry.session_id))]);
        let payload = json!({"resourceLogs": [{"resource": {"attributes": resource},
            "scopeLogs": [{"scope": {"name": semconv::SESSION_DOMAIN, "version": semconv::VERSION}, "logRecords": [record]}]}]});
        TelemetryLog::new(&self.state, entry.device_id).append(
            Signal::Logs,
            vec![payload],
            entry.ts_ms,
        )?;
        self.written.insert(entry.event_id);
        Ok(Some(event))
    }
}

fn mapped_name(entry: &LogEntry, phase: bool) -> Option<&'static str> {
    if entry.body["state_accepted"] == false {
        return None;
    }
    Some(match entry.event_type.as_str() {
        "session.created" => semconv::SESSION_CREATED,
        "session.running" => semconv::SESSION_RUNNING,
        "session.blocked" => semconv::SESSION_BLOCKED,
        "session.finished" | "done" => semconv::SESSION_FINISHED,
        "session.failed" => semconv::SESSION_FAILED,
        "session.reopened" => semconv::SESSION_REOPENED,
        "session.renamed" => semconv::SESSION_RENAMED,
        "session.orphaned" => semconv::SESSION_ORPHANED,
        "session.resumed" => semconv::SESSION_RESUMED,
        "session.failed_over" => semconv::SESSION_FAILED_OVER,
        "session.claim_rejected" => semconv::SESSION_CLAIM_REJECTED,
        "message.sent" => semconv::MESSAGE_SENT,
        "message.delivered" => semconv::MESSAGE_DELIVERED,
        "message.read" => semconv::MESSAGE_READ,
        "edge.added" | "session.depends_on" | "session.block_on" => semconv::EDGE_ADDED,
        "edge.removed" => semconv::EDGE_REMOVED,
        "edge.action_registered" => semconv::TRIGGER_REGISTERED,
        "edge.action_removed" => semconv::TRIGGER_REMOVED,
        "edge.action_fired" => semconv::TRIGGER_FIRED,
        "edge.action_failed" => semconv::TRIGGER_FAILED,
        "turn.started" => semconv::TURN_STARTED,
        "turn.ended" if entry.body["stop_reply"]["decision"] != "block" => semconv::TURN_ENDED,
        "turn.continued" => semconv::TURN_CONTINUED,
        "turn.cancelled" => semconv::TURN_CANCELLED,
        "tool.pre" => semconv::TOOL_PRE,
        "tool.post" => semconv::TOOL_POST,
        "agent.waiting" => semconv::AGENT_WAITING,
        "subagent.started" => semconv::SUBAGENT_STARTED,
        "subagent.stopped" => semconv::SUBAGENT_STOPPED,
        "exit" => semconv::SESSION_EXIT,
        "process.exited" if phase => semconv::PHASE_FINISHED,
        "process.exited" => semconv::SESSION_EXIT,
        "phase.started" | "phase.spawned" => semconv::PHASE_STARTED,
        "phase.finished" => semconv::PHASE_FINISHED,
        "runner.usage.updated" => semconv::RUNNER_USAGE_UPDATED,
        _ => return None,
    })
}

pub(crate) fn event_attributes(entry: &LogEntry, name: &str, phase: Option<&Value>) -> Vec<Value> {
    let mut attributes = vec![
        attribute(semconv::EVENT_ID, json!(entry.event_id)),
        attribute(semconv::SEMCONV_VERSION, json!(semconv::VERSION)),
        attribute(semconv::EVENT_DOMAIN, json!(semconv::SESSION_DOMAIN)),
        attribute(semconv::EVENT_NAME, json!(name)),
        attribute(semconv::SESSION_EPOCH, json!(entry.epoch)),
        attribute(semconv::SESSION_SEQ, json!(entry.seq)),
    ];
    if matches!(
        name,
        semconv::SESSION_CREATED
            | semconv::SESSION_RUNNING
            | semconv::SESSION_BLOCKED
            | semconv::SESSION_FINISHED
            | semconv::SESSION_FAILED
            | semconv::SESSION_REOPENED
            | semconv::SESSION_ORPHANED
            | semconv::SESSION_RESUMED
            | semconv::SESSION_FAILED_OVER
    ) {
        attributes.push(attribute(
            semconv::SESSION_STATE,
            json!(name.strip_prefix("ws.session.").unwrap()),
        ));
    }
    for (key, source) in [
        (semconv::SESSION_NAME, "name"),
        (semconv::SESSION_KIND, "kind"),
        (semconv::REASON, "reason"),
    ] {
        let applies = match key {
            semconv::SESSION_NAME => {
                matches!(name, semconv::SESSION_CREATED | semconv::SESSION_RENAMED)
            }
            semconv::SESSION_KIND => name == semconv::SESSION_CREATED,
            _ => true,
        };
        if applies && let Some(value) = entry.body[source].as_str() {
            attributes.push(attribute(key, json!(value)));
        }
    }
    if name.starts_with("ws.session.message.") {
        let sent = name == semconv::MESSAGE_SENT;
        let id = if sent {
            json!(entry.event_id)
        } else {
            entry.body["message_id"].clone()
        };
        attributes.push(attribute(semconv::MESSAGE_ID, id));
        for (key, value) in [
            (
                semconv::MESSAGE_FROM,
                entry
                    .body
                    .get("from_session")
                    .cloned()
                    .or_else(|| sent.then(|| json!(entry.session_id))),
            ),
            (
                semconv::MESSAGE_TO,
                entry
                    .body
                    .get("to_session")
                    .or_else(|| entry.body.get("recipient"))
                    .cloned(),
            ),
            (
                semconv::MESSAGE_PAIR_SEQ,
                entry.body.get("pair_seq").cloned(),
            ),
        ] {
            if let Some(value) = value.filter(|v| !v.is_null()) {
                attributes.push(attribute(key, value));
            }
        }
    }
    if name.starts_with("ws.session.edge.") {
        attributes.push(attribute(
            semconv::EDGE_ID,
            if name == semconv::EDGE_ADDED {
                json!(entry.event_id)
            } else {
                entry.body["edge_id"].clone()
            },
        ));
        let kind = match entry.event_type.as_str() {
            "session.depends_on" => json!("depends_on"),
            "session.block_on" => json!("block_on"),
            _ => entry.body["kind"].clone(),
        };
        for (key, value) in [
            (semconv::EDGE_KIND, kind),
            (semconv::EDGE_TARGET, entry.body["target"].clone()),
        ] {
            if !value.is_null() {
                attributes.push(attribute(key, value));
            }
        }
    }
    if name.starts_with("ws.session.trigger.") {
        attributes.push(attribute(
            semconv::TRIGGER_ID,
            entry
                .body
                .get("trigger_id")
                .or_else(|| entry.body.get("id"))
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    if matches!(name, semconv::PHASE_STARTED | semconv::PHASE_FINISHED) {
        for (key, source) in [
            (semconv::PHASE_ID, "phase"),
            (semconv::PHASE_ATTEMPT, "attempt"),
        ] {
            if let Some(value) = entry
                .body
                .get(source)
                .or_else(|| phase.and_then(|p| p.get(source)))
            {
                let value = if key == semconv::PHASE_ID {
                    json!(
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    )
                } else {
                    value.clone()
                };
                attributes.push(attribute(key, value));
            }
        }
    }
    if name == semconv::RUNNER_USAGE_UPDATED {
        for key in [
            semconv::RUNNER,
            semconv::RUNNER_ACCOUNT_ID,
            semconv::RUNNER_USAGE_BUCKET,
            semconv::RUNNER_USAGE_WINDOW,
            semconv::RUNNER_USAGE_WINDOW_DURATION,
            semconv::RUNNER_USAGE_WINDOW_START,
            semconv::RUNNER_USAGE_WINDOW_RESET,
            semconv::RUNNER_USAGE_SOURCE,
            semconv::RUNNER_USAGE_SOURCE_NAME,
            semconv::RUNNER_USAGE_OBSERVED_AT,
            semconv::RUNNER_USAGE_COVERAGE,
            semconv::RUNNER_USAGE_MODEL,
            semconv::RUNNER_USAGE_USED_RATIO,
            semconv::RUNNER_USAGE_REMAINING_RATIO,
            semconv::RUNNER_USAGE_TOKENS,
        ] {
            if let Some(value) = entry.body.get(key).filter(|value| !value.is_null()) {
                attributes.push(attribute(key, value.clone()));
            }
        }
    }
    attributes
}

/// Returns the `ws.project` value of a `session.created` body: `project`, else `workspace_id`.
pub(crate) fn session_project(body: &Value) -> Option<&str> {
    body["project"]
        .as_str()
        .or_else(|| body["workspace_id"].as_str())
}

fn timestamp(entry: &LogEntry) -> u64 {
    if entry.event_type == "runner.usage.updated"
        && let Some(observed) = entry.body[semconv::RUNNER_USAGE_OBSERVED_AT].as_u64()
    {
        return observed.saturating_mul(1_000_000);
    }
    entry.body["observed_at_us"]
        .as_u64()
        .map(|us| us.saturating_mul(1000))
        .unwrap_or_else(|| entry.ts_ms.saturating_mul(1_000_000))
}

fn attribute(key: &str, value: Value) -> Value {
    json!({"key": key, "value": any_value(&value)})
}

/// Removes personal data always, and content fields when content export is disabled.
/// `parent` is the key of the enclosing object, for keys that are content only in context.
fn redact_body(value: &mut Value, parent: Option<&str>, content: bool) {
    match value {
        Value::Object(fields) => {
            fields.retain(|key, _| key != "user.email" && (content || !is_content(key, parent)));
            for (key, value) in fields.iter_mut() {
                redact_body(value, Some(key), content);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_body(value, parent, content);
            }
        }
        _ => {}
    }
}

fn is_content(key: &str, parent: Option<&str>) -> bool {
    matches!(
        key,
        "body"
            | "text"
            | "prompt"
            | "template"
            | "arguments"
            | "output"
            | "tool_input"
            | "tool_response"
            | "command"
            | "argv"
            | "args"
    ) || (parent == Some("stop_reply") && key == "reason")
}

fn any_value(value: &Value) -> Value {
    match value {
        Value::Null => json!({}),
        Value::Bool(value) => json!({"boolValue":value}),
        Value::Number(value) if value.is_i64() || value.is_u64() => {
            json!({"intValue":value.to_string()})
        }
        Value::Number(value) => json!({"doubleValue":value}),
        Value::String(value) => json!({"stringValue":value}),
        Value::Array(values) => {
            json!({"arrayValue":{"values":values.iter().map(any_value).collect::<Vec<_>>()}})
        }
        Value::Object(values) => {
            json!({"kvlistValue":{"values":values.iter().map(|(key,value)| attribute(key, value.clone())).collect::<Vec<_>>()}})
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sessions::{CreateSession, Sessions};
    use serde_json::{Value, json};
    use ws_core::{
        log::SessionLog,
        telemetry_log::{Signal, TelemetryLog},
    };

    const MAPPINGS: &[(&str, &str)] = &[
        ("session.created", "ws.session.created"),
        ("session.running", "ws.session.running"),
        ("session.blocked", "ws.session.blocked"),
        ("session.finished", "ws.session.finished"),
        ("session.failed", "ws.session.failed"),
        ("session.reopened", "ws.session.reopened"),
        ("session.renamed", "ws.session.renamed"),
        ("session.orphaned", "ws.session.orphaned"),
        ("session.resumed", "ws.session.resumed"),
        ("session.failed_over", "ws.session.failed_over"),
        ("session.claim_rejected", "ws.session.claim_rejected"),
        ("message.sent", "ws.session.message.sent"),
        ("message.delivered", "ws.session.message.delivered"),
        ("message.read", "ws.session.message.read"),
        ("edge.added", "ws.session.edge.added"),
        ("edge.removed", "ws.session.edge.removed"),
        ("edge.action_registered", "ws.session.trigger.registered"),
        ("edge.action_removed", "ws.session.trigger.removed"),
        ("edge.action_fired", "ws.session.trigger.fired"),
        ("edge.action_failed", "ws.session.trigger.failed"),
        ("turn.started", "ws.session.turn.started"),
        ("turn.ended", "ws.session.turn.ended"),
        ("turn.continued", "ws.session.turn.continued"),
        ("turn.cancelled", "ws.session.turn.cancelled"),
        ("tool.pre", "ws.session.tool.pre"),
        ("tool.post", "ws.session.tool.post"),
        ("agent.waiting", "ws.session.agent.waiting"),
        ("subagent.started", "ws.session.subagent.started"),
        ("subagent.stopped", "ws.session.subagent.stopped"),
        ("exit", "ws.session.exit"),
        ("phase.started", "phase.started"),
        ("phase.finished", "phase.finished"),
        ("runner.usage.updated", "ws.runner.usage.updated"),
    ];

    #[test]
    fn every_catalog_mapping_emits_one_otlp_log_with_source_identity() {
        let document = include_str!("../../../../docs/ws-v6/semconv-v1.md");
        let rows = document
            .split("## Log records and span events")
            .nth(1)
            .unwrap()
            .split("## Runner usage attributes")
            .next()
            .unwrap();
        let names: std::collections::BTreeSet<_> = rows
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .map(|line| line.split('`').next().unwrap())
            .collect();
        assert_eq!(names, MAPPINGS.iter().map(|(_, name)| *name).collect());
        for &(source_type, expected) in MAPPINGS {
            let sandbox = tempfile::tempdir().unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            drop(sessions);
            let source = SessionLog::new(sandbox.path(), session.id, session.device_id);
            let entry = if source_type == "session.created" {
                source.read().unwrap().entries[0].clone()
            } else {
                source
                    .append(
                        0,
                        source_type,
                        json!({"observed_at_us":123456,
                    "name":"sample", "reason":"example", "kind":"depends_on",
                    "body":"message", "edges":[],
                    "target":session.id, "trigger_id":session.id,
                    "message_id":session.id, "from_session":session.id,
                    "to_session":session.id, "pair_seq":7, "edge_id":session.id,
                    "tool_use_id":"tool", "phase":"44", "attempt":1}),
                    )
                    .unwrap()
            };
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let records = log.read().unwrap();
            let mapped: Vec<_> = records
                .iter()
                .filter(|r| r.signal == Signal::Logs)
                .map(|r| &r.payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0])
                .filter(|r| attr(r, "event_id")["stringValue"] == entry.event_id.to_string())
                .collect();
            assert_eq!(mapped.len(), 1, "{source_type}");
            let record = mapped[0];
            assert_eq!(
                record["traceId"],
                format!("{:032x}", u128::from(session.id))
            );
            if source_type != "session.created" {
                assert!(
                    !record["attributes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a["key"] == "ws.session.kind")
                );
            }
            assert_eq!(attr(record, "event.name")["stringValue"], expected);
            assert_eq!(attr(record, "event.domain")["stringValue"], "ws.session");
            assert_eq!(attr(record, "ws.semconv.version")["stringValue"], "1");
            assert_eq!(attr(record, "ws.session.epoch")["intValue"], "0");
            assert_eq!(
                attr(record, "ws.session.seq")["intValue"],
                entry.seq.to_string()
            );
            assert_eq!(
                record["body"]["kvlistValue"]["values"][0]["key"],
                "event.name"
            );
            let expected_ns = if source_type == "session.created" {
                entry.ts_ms * 1_000_000
            } else {
                123456000
            };
            assert_eq!(record["timeUnixNano"], expected_ns.to_string());
            for record in &records {
                if record.signal == Signal::Logs {
                    serde_json::from_value::<
                        opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest,
                    >(record.payload.clone())
                    .unwrap();
                }
            }
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            assert_eq!(log.read().unwrap(), records, "{source_type}: replay");
        }
    }

    #[test]
    fn events_attach_to_open_session_spans_before_closure_without_duplicates() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let other = sessions.create(CreateSession::default()).unwrap();
        let mut entries = Vec::new();
        for (time, source, body) in [
            (100, "turn.started", json!({"turn_id":"turn"})),
            (200, "tool.pre", json!({"tool_use_id":"tool"})),
            (250, "message.read", json!({"message_id":session.id})),
            (260, "edge.action_fired", json!({"trigger_id":session.id})),
            (
                300,
                "tool.post",
                json!({"tool_use_id":"tool", "tool_status":1}),
            ),
            (350, "session.renamed", json!({"name":"new-name"})),
            (400, "turn.ended", json!({"turn_id":"turn"})),
            (450, "message.read", json!({"message_id":session.id})),
        ] {
            let mut body = body;
            body["observed_at_us"] = json!(time);
            if time == 260 {
                sessions
                    .append_entry(other.id, "agent.waiting", json!({"observed_at_us":255}))
                    .unwrap();
            }
            entries.push(sessions.append_entry(session.id, source, body).unwrap());
        }
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let records = log.read().unwrap();
        let spans: Vec<_> = records
            .iter()
            .filter(|r| r.signal == Signal::Traces)
            .map(|r| &r.payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0])
            .collect();
        assert_eq!(spans.len(), 2);
        for (span_name, indices) in [
            ("ws.session.tool", vec![1, 2, 3, 4]),
            ("ws.session.turn", vec![0, 1, 2, 3, 4, 5, 6]),
        ] {
            let span = spans.iter().find(|s| s["name"] == span_name).unwrap();
            let events = span["events"].as_array().unwrap();
            assert_eq!(events.len(), indices.len(), "{span_name}");
            for (event, index) in events.iter().zip(indices) {
                assert_eq!(
                    attr(event, "event_id")["stringValue"],
                    entries[index].event_id.to_string()
                );
                assert_eq!(attr(event, "event.domain")["stringValue"], "ws.session");
                assert_eq!(attr(event, "ws.semconv.version")["stringValue"], "1");
                let record = records
                    .iter()
                    .filter(|r| r.signal == Signal::Logs)
                    .map(|r| &r.payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0])
                    .find(|r| {
                        attr(r, "event_id")["stringValue"] == entries[index].event_id.to_string()
                    })
                    .unwrap();
                assert_eq!(event["name"], attr(record, "event.name")["stringValue"]);
                assert_eq!(event["attributes"], record["attributes"]);
            }
        }
        drop(sessions);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        assert_eq!(log.read().unwrap(), records);
    }

    #[test]
    fn session_content_obeys_the_export_policy() {
        for content in [false, true] {
            let sandbox = tempfile::tempdir().unwrap();
            std::fs::write(
                sandbox.path().join("config.toml"),
                format!("[telemetry]\ncontent = {content}\n"),
            )
            .unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            sessions.append_entry(session.id, "message.sent", json!({
                "recipient":session.id, "text":"PRIVATE-MESSAGE", "pair_seq":1,
                "nested":{"user.email":"PRIVATE-EMAIL", "items":[{"output":"PRIVATE-OUTPUT", "count":2}]}
            })).unwrap();
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let records = log.read().unwrap();
            let record = &records.last().unwrap().payload;
            let encoded = record.to_string();
            assert!(!encoded.contains("PRIVATE-EMAIL"));
            assert_eq!(encoded.contains("PRIVATE-MESSAGE"), content);
            assert_eq!(encoded.contains("PRIVATE-OUTPUT"), content);
            assert!(encoded.contains("count"));
            assert!(encoded.contains("intValue"));
        }
    }

    #[test]
    fn stop_reply_reasons_and_commands_obey_the_export_policy() {
        for content in [false, true] {
            let sandbox = tempfile::tempdir().unwrap();
            std::fs::write(
                sandbox.path().join("config.toml"),
                format!("[telemetry]\ncontent = {content}\n"),
            )
            .unwrap();
            let device = ws_core::device::Device::init(sandbox.path(), "test-device").unwrap();
            let session = ws_core::ids::Ulid::new();
            let source = SessionLog::new(sandbox.path(), session, device.device_id);
            source
                .append(
                    0,
                    "session.created",
                    json!({"alias":1,"kind":"headless",
                    "command":["claude","-p","PRIVATE-PROMPT"],"cwd":"/tmp"}),
                )
                .unwrap();
            source
                .append(
                    0,
                    "turn.continued",
                    json!({"stop_reply":
                    {"decision":"block","reason":"PRIVATE-REASON"}}),
                )
                .unwrap();
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            let records = TelemetryLog::new(sandbox.path(), device.device_id)
                .read()
                .unwrap();
            assert_eq!(records.len(), 2);
            let encoded =
                serde_json::to_string(&records.iter().map(|r| &r.payload).collect::<Vec<_>>())
                    .unwrap();
            assert_eq!(encoded.contains("PRIVATE-PROMPT"), content);
            assert_eq!(encoded.contains("PRIVATE-REASON"), content);
            assert!(encoded.contains("block"));
            assert!(encoded.contains("/tmp"));
        }
    }

    #[test]
    fn the_project_resource_falls_back_to_the_workspace_id() {
        let sandbox = tempfile::tempdir().unwrap();
        let device = ws_core::device::Device::init(sandbox.path(), "test-device").unwrap();
        let session = ws_core::ids::Ulid::new();
        let source = SessionLog::new(sandbox.path(), session, device.device_id);
        source
            .append(
                0,
                "session.created",
                json!({"alias":1,"workspace_id":"workspace"}),
            )
            .unwrap();
        source.append(0, "session.running", json!({})).unwrap();
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        let records = TelemetryLog::new(sandbox.path(), device.device_id)
            .read()
            .unwrap();
        assert_eq!(records.len(), 2);
        for record in records {
            assert_eq!(
                attr(&record.payload["resourceLogs"][0]["resource"], "ws.project")["stringValue"],
                "workspace"
            );
        }
    }

    #[test]
    fn aliases_phase_exits_and_resource_metadata_keep_their_semantics() {
        for phase in [false, true] {
            let sandbox = tempfile::tempdir().unwrap();
            let device = ws_core::device::Device::init(sandbox.path(), "test-device").unwrap();
            let session = ws_core::ids::Ulid::new();
            let parent = ws_core::ids::Ulid::new();
            let source = SessionLog::new(sandbox.path(), session, device.device_id);
            let mut metadata = json!({"alias":1,"name":"runner","kind":"headless",
                "parent_session_id":parent,"project":"sample"});
            if phase {
                metadata["phase"] = json!(44);
                metadata["attempt"] = json!(2);
            }
            source.append(0, "session.created", metadata).unwrap();
            let mut entries = Vec::new();
            for (source_type, expected, body) in [
                ("done", "ws.session.finished", json!({})),
                (
                    "session.depends_on",
                    "ws.session.edge.added",
                    json!({"target":parent}),
                ),
                (
                    "session.block_on",
                    "ws.session.edge.added",
                    json!({"target":parent}),
                ),
                (
                    "message.sent",
                    "ws.session.message.sent",
                    json!({"recipient":parent,"text":"message","pair_seq":1}),
                ),
                (
                    "phase.spawned",
                    "phase.started",
                    json!({"phase":44,"attempt":2}),
                ),
                (
                    "process.exited",
                    if phase {
                        "phase.finished"
                    } else {
                        "ws.session.exit"
                    },
                    json!({"code":0}),
                ),
            ] {
                entries.push((source.append(0, source_type, body).unwrap(), expected));
            }
            for source_type in ["api.called", "hook.received", "turn.started"] {
                source
                    .append(0, source_type, json!({"state_accepted":false}))
                    .unwrap();
            }
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            let records = TelemetryLog::new(sandbox.path(), device.device_id)
                .read()
                .unwrap();
            assert_eq!(records.len(), 7);
            for stored in &records {
                let resource = &stored.payload["resourceLogs"][0]["resource"];
                assert_eq!(
                    attr(resource, "ws.session.id")["stringValue"],
                    session.to_string()
                );
                assert_eq!(
                    attr(resource, "ws.parent_session.id")["stringValue"],
                    parent.to_string()
                );
                assert_eq!(
                    attr(resource, "ws.device.id")["stringValue"],
                    device.device_id.to_string()
                );
                assert_eq!(attr(resource, "ws.project")["stringValue"], "sample");
            }
            for ((entry, expected), stored) in entries.iter().zip(&records[1..]) {
                let record = &stored.payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
                assert_eq!(attr(record, "event.name")["stringValue"], *expected);
                assert_eq!(
                    attr(record, "event_id")["stringValue"],
                    entry.event_id.to_string()
                );
                match entry.event_type.as_str() {
                    "done" => {
                        assert_eq!(attr(record, "ws.session.state")["stringValue"], "finished")
                    }
                    "session.depends_on" | "session.block_on" => {
                        assert_eq!(
                            attr(record, "ws.session.edge.id")["stringValue"],
                            entry.event_id.to_string()
                        );
                        assert_eq!(
                            attr(record, "ws.session.edge.kind")["stringValue"],
                            if entry.event_type == "session.depends_on" {
                                "depends_on"
                            } else {
                                "block_on"
                            }
                        );
                    }
                    "message.sent" => {
                        assert_eq!(
                            attr(record, "ws.session.message.id")["stringValue"],
                            entry.event_id.to_string()
                        );
                        assert_eq!(
                            attr(record, "ws.session.message.from")["stringValue"],
                            session.to_string()
                        );
                        assert_eq!(
                            attr(record, "ws.session.message.to")["stringValue"],
                            parent.to_string()
                        );
                        assert_eq!(attr(record, "ws.session.message.pair_seq")["intValue"], "1");
                    }
                    "phase.spawned" | "process.exited" if *expected != "ws.session.exit" => {
                        assert_eq!(attr(record, "ws.phase.id")["stringValue"], "44");
                        assert_eq!(attr(record, "ws.phase.attempt")["intValue"], "2");
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn imported_events_retain_the_source_device_resource() {
        let sandbox = tempfile::tempdir().unwrap();
        let local = ws_core::device::Device::init(sandbox.path(), "local").unwrap();
        let source_device = ws_core::ids::Ulid::new();
        let session = ws_core::ids::Ulid::new();
        let source = SessionLog::new(sandbox.path(), session, source_device);
        source
            .append(0, "session.created", json!({"alias":1}))
            .unwrap();
        source.append(0, "session.running", json!({})).unwrap();
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        let records = TelemetryLog::new(sandbox.path(), local.device_id)
            .read()
            .unwrap();
        assert_eq!(records.len(), 2);
        for record in records {
            assert_eq!(
                attr(
                    &record.payload["resourceLogs"][0]["resource"],
                    "ws.device.id"
                )["stringValue"],
                source_device.to_string()
            );
        }
    }

    fn attr<'a>(value: &'a Value, key: &str) -> &'a Value {
        &value["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["key"] == key)
            .unwrap_or_else(|| panic!("The attribute {key} is missing."))["value"]
    }
}
