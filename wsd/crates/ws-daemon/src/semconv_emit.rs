//! OTLP log records from durable session events.

use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::Path,
};
use ws_core::{
    ids::{DeviceId, EventId, SessionId},
    log::LogEntry,
    semconv,
    telemetry_log::{Signal, TelemetryLog},
};

pub(crate) struct Emitter {
    log: TelemetryLog,
    written: HashSet<EventId>,
    resources: HashMap<SessionId, Vec<Value>>,
    phases: HashMap<SessionId, Value>,
}

impl Emitter {
    pub(crate) fn open(state: &Path, device: DeviceId) -> io::Result<Self> {
        let log = TelemetryLog::new(state, device);
        let mut written = HashSet::new();
        for record in log.read()?.into_iter().filter(|r| r.signal == Signal::Logs) {
            for resource in record.payload["resourceLogs"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for scope in resource["scopeLogs"].as_array().into_iter().flatten() {
                    if scope["scope"]["name"] != "ws.session" {
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
                            written.insert(id);
                        }
                    }
                }
            }
        }
        Ok(Self {
            log,
            written,
            resources: HashMap::new(),
            phases: HashMap::new(),
        })
    }

    pub(crate) fn observe(&mut self, entry: &LogEntry) -> io::Result<()> {
        if entry.event_type == "session.created" {
            let mut resource = vec![attribute(semconv::SESSION_ID, json!(entry.session_id))];
            for (key, source) in [
                (semconv::PARENT_SESSION_ID, "parent_session_id"),
                (semconv::PROJECT, "project"),
            ] {
                if let Some(value) = entry.body[source].as_str() {
                    resource.push(attribute(key, json!(value)));
                }
            }
            self.resources.insert(entry.session_id, resource);
            if entry.body.get("phase").is_some() {
                self.phases.insert(entry.session_id, entry.body.clone());
            }
        }
        let phase = self.phases.get(&entry.session_id);
        let Some(name) = mapped_name(entry, phase.is_some()) else {
            return Ok(());
        };
        if self.written.contains(&entry.event_id) {
            return Ok(());
        }
        let attributes = event_attributes(entry, name, phase);
        let record = json!({"timeUnixNano": timestamp(entry).to_string(),
            "eventName": name, "attributes": attributes,
            "body": {"kvlistValue": {"values": [
                {"key": semconv::EVENT_NAME, "value": {"stringValue": name}},
                {"key": "body", "value": any_value(&entry.body)}]}}});
        let resource = self
            .resources
            .get(&entry.session_id)
            .cloned()
            .unwrap_or_else(|| vec![attribute(semconv::SESSION_ID, json!(entry.session_id))]);
        let payload = json!({"resourceLogs": [{"resource": {"attributes": resource},
            "scopeLogs": [{"scope": {"name": "ws.session", "version": semconv::VERSION}, "logRecords": [record]}]}]});
        self.log.append(Signal::Logs, vec![payload], entry.ts_ms)?;
        self.written.insert(entry.event_id);
        Ok(())
    }
}

pub(crate) fn mapped_name(entry: &LogEntry, phase: bool) -> Option<&'static str> {
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
        if let Some(value) = entry.body[source].as_str() {
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
    attributes
}

pub(crate) fn timestamp(entry: &LogEntry) -> u64 {
    entry.body["observed_at_us"]
        .as_u64()
        .map(|us| us.saturating_mul(1000))
        .unwrap_or_else(|| entry.ts_ms.saturating_mul(1_000_000))
}

fn attribute(key: &str, value: Value) -> Value {
    json!({"key": key, "value": any_value(&value)})
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

    fn attr<'a>(value: &'a Value, key: &str) -> &'a Value {
        &value["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["key"] == key)
            .unwrap_or_else(|| panic!("The attribute {key} is missing."))["value"]
    }
}
