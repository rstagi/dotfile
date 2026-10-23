//! Canonical turn, tool and model spans from durable events.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};
use ws_core::{
    ids::{DeviceId, EventId, SessionId},
    log::LogEntry,
    semconv,
    telemetry_log::{Signal, TelemetryCursor, TelemetryLog, TelemetryRecord},
};

pub(crate) type SharedSpans = Arc<Mutex<Spans>>;

pub(crate) struct Spans {
    state: PathBuf,
    log: TelemetryLog,
    emitter: crate::semconv_emit::Emitter,
    native_tracing: bool,
    written: HashSet<(String, String)>,
    sessions: HashMap<SessionId, SessionSpans>,
    cursor: TelemetryCursor,
    /// Unjoined models per session. A join depends only on its session's spans and native streams.
    pending_models: HashMap<SessionId, Vec<PendingModel>>,
    native_streams: Vec<NativeStream>,
    /// Sessions whose spans or native streams changed since the last synthesis.
    dirty: HashSet<SessionId>,
    /// Turn links per target session. Each read parses only the log bytes after the last read.
    turn_links: RefCell<HashMap<SessionId, TurnLinks>>,
    #[cfg(test)]
    scanned: usize,
    #[cfg(test)]
    joins: usize,
}

#[derive(Default)]
struct TurnLinks {
    read_bytes: u64,
    links: Vec<Value>,
}

/// A model record whose turn or timing was not yet known when it was read.
struct PendingModel {
    event_id: EventId,
    received_at_ms: u64,
    ordinal: usize,
    session: SessionId,
    time: u64,
    /// Claude records carry a measured duration. Codex records do not.
    duration_ms: Option<u64>,
    record: Value,
}

struct NativeStream {
    session: SessionId,
    start: u64,
    end: u64,
}

#[derive(Clone, Default)]
struct SessionSpans {
    resource: Vec<Value>,
    exited: bool,
    relations: HashMap<String, SessionId>,
    turn: Option<Turn>,
    closed_turns: Vec<Turn>,
    tool_results: HashMap<String, u64>,
}

#[derive(Clone)]
struct Turn {
    source_id: Option<String>,
    span: Value,
    tools: HashMap<String, Value>,
    seen_tools: HashSet<String>,
    boundaries: Vec<u64>,
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
                Some(semconv::TURN_SPAN | semconv::TOOL_SPAN | semconv::MODEL_CALL_SPAN)
            ) {
                written.insert((
                    span["traceId"].as_str().unwrap_or("").into(),
                    span["spanId"].as_str().unwrap_or("").into(),
                ));
            }
        }
        Ok(Self {
            state: state.into(),
            native_tracing: ws_core::config::Config::load(state)?
                .telemetry
                .native_tracing,
            log,
            emitter: crate::semconv_emit::Emitter::open(state, device)?,
            written,
            sessions: HashMap::new(),
            cursor: TelemetryCursor::default(),
            pending_models: HashMap::new(),
            native_streams: Vec::new(),
            dirty: HashSet::new(),
            turn_links: RefCell::default(),
            #[cfg(test)]
            scanned: 0,
            #[cfg(test)]
            joins: 0,
        })
    }

    /// Consumes each durable entry once. Replay uses the same source-derived span ids.
    pub(crate) fn observe(&mut self, entry: &LogEntry) -> io::Result<()> {
        if self.replay(entry)? {
            self.synthesize_models()?;
        }
        Ok(())
    }

    /// Consumes one entry without model synthesis. Recovery replays every entry, then synthesizes once.
    /// Returns true when the entry changed the session's spans.
    pub(crate) fn replay(&mut self, entry: &LogEntry) -> io::Result<bool> {
        let emitted_event = self.emitter.observe(entry)?;
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
            return Ok(false);
        }
        if emitted_event.is_none()
            && !matches!(
                event,
                "session.created"
                    | "edge.added"
                    | "edge.removed"
                    | "session.depends_on"
                    | "turn.started"
                    | "turn.ended"
                    | "turn.continued"
                    | "turn.cancelled"
                    | "tool.pre"
                    | "tool.post"
                    | "agent.waiting"
                    | "subagent.started"
                    | "subagent.stopped"
                    | "exit"
                    | "process.exited"
                    | "session.failed"
                    | "session.finished"
                    | "session.orphaned"
                    | "done"
            )
        {
            return Ok(false);
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
                    | "agent.waiting"
                    | "subagent.started"
                    | "subagent.stopped"
            )
        {
            return Ok(false);
        }
        // A failed append leaves the caller's rebuild flag set. The rebuild replaces this memory,
        // so the state moves out of the map instead of a copy for each entry.
        let mut state = self.sessions.remove(&entry.session_id).unwrap_or_default();
        if let Some(event) = emitted_event
            && let Some(turn) = state.turn.as_mut()
            && turn
                .source_id
                .as_deref()
                .zip(turn_id(entry))
                .is_none_or(|(current, incoming)| current == incoming)
            && timestamp(entry) >= number(&turn.span["startTimeUnixNano"]).unwrap()
        {
            push_event(&mut turn.span, event.clone());
            for (id, tool) in &mut turn.tools {
                if timestamp(entry) >= number(&tool["startTimeUnixNano"]).unwrap()
                    && (entry.event_type != "tool.post" || entry.body["tool_use_id"] == id.as_str())
                {
                    push_event(tool, event.clone());
                }
            }
        }
        let result = entry.body["callback_id"]
            .as_str()
            .and_then(|id| state.tool_results.remove(id));
        if matches!(
            event,
            "exit"
                | "process.exited"
                | "session.failed"
                | "session.finished"
                | "session.orphaned"
                | "done"
        ) {
            state.exited = true;
        } else if event == "turn.started" {
            state.exited = false;
        }
        let mut finished = Vec::new();
        if event == "session.created" {
            state.resource = vec![attribute(semconv::SESSION_ID, entry.session_id.to_string())];
            if let Some(parent) = entry.body["parent_session_id"].as_str() {
                state
                    .resource
                    .push(attribute(semconv::PARENT_SESSION_ID, parent));
            }
            if let Some(project) = crate::semconv_emit::session_project(&entry.body) {
                state.resource.push(attribute(semconv::PROJECT, project));
            }
            if let Some(parent) = entry.body["parent_session_id"]
                .as_str()
                .and_then(|s| s.parse().ok())
            {
                state.relations.insert("parent".into(), parent);
            }
        } else if event == "edge.added" || event == "session.depends_on" {
            if (event == "session.depends_on"
                || matches!(entry.body["kind"].as_str(), Some("parent" | "depends_on")))
                && let Some(target) = entry.body["target"].as_str().and_then(|s| s.parse().ok())
            {
                state.relations.insert(entry.event_id.to_string(), target);
            }
        } else if event == "edge.removed" {
            if let Some(edge) = entry.body["edge_id"].as_str() {
                state.relations.remove(edge);
            }
        } else if event == "turn.started" {
            close_turn(&mut state, entry, true, &mut finished);
            let id = turn_id(entry)
                .map(str::to_owned)
                .unwrap_or_else(|| entry.event_id.to_string());
            let mut span = new_span(entry, semconv::TURN_SPAN);
            span["links"] = json!(self.segment_links(&state, &span)?);
            span["attributes"]
                .as_array_mut()
                .unwrap()
                .push(attribute(semconv::TURN_ID, &id));
            state.turn = Some(Turn {
                source_id: turn_id(entry).map(str::to_owned),
                span,
                tools: HashMap::new(),
                seen_tools: HashSet::new(),
                boundaries: vec![timestamp(entry)],
            });
        } else if let Some(turn) = state.turn.as_mut() {
            let same_turn = turn
                .source_id
                .as_deref()
                .zip(turn_id(entry))
                .is_none_or(|(current, incoming)| current == incoming);
            if same_turn
                && matches!(
                    event,
                    "tool.pre"
                        | "tool.post"
                        | "turn.continued"
                        | "turn.ended"
                        | "turn.cancelled"
                        | "agent.waiting"
                        | "subagent.started"
                        | "subagent.stopped"
                )
            {
                turn.boundaries.push(timestamp(entry));
            }
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
        for span in &mut finished {
            if span["name"] == semconv::TURN_SPAN {
                let links = self.segment_links(&state, span)?;
                let existing = span["links"].as_array_mut().unwrap();
                for link in links {
                    if !existing.contains(&link) {
                        existing.push(link);
                    }
                }
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
        self.dirty.insert(entry.session_id);
        Ok(true)
    }

    /// Joins durable agent records to observed turn intervals. Reads only records after the cursor and retains unjoined models.
    pub(crate) fn synthesize_models(&mut self) -> io::Result<()> {
        let tail = self.log.read_after(self.cursor)?;
        #[cfg(test)]
        {
            self.scanned += tail.records.len();
        }
        if tail.restarted {
            self.pending_models.clear();
            self.native_streams.clear();
        }
        self.cursor = tail.cursor;
        for record in &tail.records {
            match record.signal {
                Signal::Logs => {
                    for model in model_candidates(record) {
                        self.dirty.insert(model.session);
                        self.pending_models
                            .entry(model.session)
                            .or_default()
                            .push(model);
                    }
                }
                Signal::Traces => {
                    for stream in native_streams(record) {
                        self.dirty.insert(stream.session);
                        self.native_streams.push(stream);
                    }
                }
                Signal::Metrics => {}
            }
        }
        // Only a changed session can join a model that an earlier pass retained.
        let dirty: Vec<_> = self.dirty.drain().collect();
        for (index, session) in dirty.iter().enumerate() {
            let Some(models) = self.pending_models.remove(session) else {
                continue;
            };
            let mut pending = models.into_iter();
            let mut retained = Vec::new();
            while let Some(model) = pending.next() {
                match self.join_model(&model) {
                    Ok(true) => {}
                    Ok(false) => retained.push(model),
                    Err(error) => {
                        retained.push(model);
                        retained.extend(pending);
                        self.pending_models.insert(*session, retained);
                        self.dirty.extend(&dirty[index..]);
                        return Err(error);
                    }
                }
            }
            if !retained.is_empty() {
                self.pending_models.insert(*session, retained);
            }
        }
        Ok(())
    }

    /// Returns false while the model's turn or timing is not yet known. Replay retains each span identity.
    fn join_model(&mut self, model: &PendingModel) -> io::Result<bool> {
        #[cfg(test)]
        {
            self.joins += 1;
        }
        let Some(state) = self.sessions.get(&model.session) else {
            return Ok(false);
        };
        let time = model.time;
        let Some(turn) = state
            .closed_turns
            .iter()
            .chain(state.turn.iter())
            .find(|turn| {
                let start = number(&turn.span["startTimeUnixNano"]).unwrap();
                start <= time && time <= *turn.boundaries.last().unwrap()
            })
        else {
            return Ok(false);
        };
        let (start, end, timing) = if let Some(duration) = model.duration_ms {
            (
                time.saturating_sub(duration.saturating_mul(1_000_000)),
                time,
                "measured",
            )
        } else if let Some((start, end)) =
            native_interval(&self.native_streams, model.session, time)
        {
            (start, end, "measured")
        } else {
            // Native exporters flush independently. Retain pending records until timing or exit.
            if self.native_tracing && !state.exited {
                return Ok(false);
            }
            let Some(window) = turn
                .boundaries
                .windows(2)
                .find(|w| w[0] <= time && time <= w[1])
            else {
                return Ok(false);
            };
            (window[0], window[1], "estimated")
        };
        let attributes = model_attributes(
            &model.record,
            model.duration_ms.is_some(),
            model.event_id,
            timing,
        );
        let span = json!({"traceId":turn.span["traceId"],
            "spanId":format!("{:016x}", (u128::from(model.event_id) as u64).wrapping_add(model.ordinal as u64 + 1).max(1)),
            "parentSpanId":turn.span["spanId"], "name":semconv::MODEL_CALL_SPAN,"kind":3,
            "startTimeUnixNano":start.to_string(),"endTimeUnixNano":end.to_string(),
            "attributes":attributes,"status":{"code":1}});
        if !self.written.contains(&identity(&span)) {
            let payload = json!({"resourceSpans":[{"resource":{"attributes":state.resource},"scopeSpans":[{"scope":{"name":"ws.spans","version":semconv::VERSION},"spans":[span]}]}]});
            self.log
                .append(Signal::Traces, vec![payload], model.received_at_ms)?;
            self.written.insert(identity(&span));
        }
        Ok(true)
    }

    fn segment_links(&self, state: &SessionSpans, current: &Value) -> io::Result<Vec<Value>> {
        let mut links = Vec::new();
        if let Some(previous) = state
            .closed_turns
            .iter()
            .rev()
            .find(|turn| turn.span["spanId"] != current["spanId"])
        {
            links
                .push(json!({"traceId":previous.span["traceId"],"spanId":previous.span["spanId"]}));
        }
        let mut targets: Vec<_> = state.relations.values().copied().collect();
        targets.sort();
        targets.dedup();
        let mut cache = self.turn_links.borrow_mut();
        for target in targets {
            let cached = cache.entry(target).or_default();
            self.read_turn_links(target, cached)?;
            links.extend(cached.links.iter().cloned());
        }
        Ok(links)
    }

    /// Appends the turn links of the log entries after the last read. Logs are append-only.
    fn read_turn_links(&self, target: SessionId, cached: &mut TurnLinks) -> io::Result<()> {
        let tail = ws_core::log::SessionLog::new(&self.state, target, DeviceId::nil())
            .read_after(cached.read_bytes)?;
        if tail.restarted {
            cached.links.clear();
        }
        cached.read_bytes = tail.offset;
        for entry in tail.entries {
            if matches!(entry.event_type.as_str(), "turn.started" | "phase.started")
                && entry.body["state_accepted"] != false
            {
                let span = new_span(&entry, semconv::TURN_SPAN);
                cached
                    .links
                    .push(json!({"traceId":span["traceId"],"spanId":span["spanId"]}));
            }
        }
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

// Native boundaries overlap canonical spans. Keep Codex transport timing for synthesis.
pub(crate) fn is_native_boundary(payload: &Value) -> bool {
    let scope = &payload["resourceSpans"][0]["scopeSpans"][0];
    let name = scope["spans"][0]["name"].as_str().unwrap_or("");
    if matches!(
        name,
        semconv::TURN_SPAN | semconv::TOOL_SPAN | semconv::MODEL_CALL_SPAN
    ) {
        return true;
    }
    match scope["scope"]["name"].as_str() {
        Some("com.anthropic.claude_code.tracing") => {
            matches!(
                name,
                "claude_code.tool" | "claude_code.tool.execution" | "claude_code.llm_request"
            )
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
    turn.boundaries.push(timestamp(entry));
    finished.push(turn.span.clone());
    state.closed_turns.push(turn);
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
    push_event(span, span_event(entry, event));
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
    json!({"name": name, "timeUnixNano": timestamp(entry).to_string(),
        "attributes": crate::semconv_emit::event_attributes(entry, name, None)})
}

fn push_event(span: &mut Value, event: Value) {
    let id = string_attribute(&event, semconv::EVENT_ID);
    let events = span["events"].as_array_mut().unwrap();
    if !events
        .iter()
        .any(|previous| string_attribute(previous, semconv::EVENT_ID) == id)
    {
        events.push(event);
    }
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

fn signal_records(payload: &Value, signal: Signal) -> impl Iterator<Item = (&Value, &Value)> {
    let (scopes, items) = match signal {
        Signal::Logs => ("scopeLogs", "logRecords"),
        Signal::Traces => ("scopeSpans", "spans"),
        Signal::Metrics => ("scopeMetrics", "metrics"),
    };
    payload[signal.resource_key()]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(move |resource| {
            resource[scopes]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(move |scope| {
                    scope[items]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(move |item| (resource, item))
                })
        })
}

fn model_attributes(model: &Value, claude: bool, event_id: EventId, timing: &str) -> Vec<Value> {
    let mut attributes = vec![
        attribute(semconv::SEMCONV_VERSION, semconv::VERSION),
        attribute(semconv::EVENT_ID, event_id.to_string()),
        attribute(semconv::TIMING, timing),
        attribute(semconv::GEN_AI_OPERATION_NAME, "chat"),
        attribute(
            semconv::GEN_AI_PROVIDER_NAME,
            if claude { "anthropic" } else { "openai" },
        ),
    ];
    for producer in model["attributes"].as_array().into_iter().flatten() {
        if matches!(
            producer["key"].as_str(),
            Some(
                "model"
                    | "request_id"
                    | "input_tokens"
                    | "output_tokens"
                    | "cache_read_tokens"
                    | "cache_creation_tokens"
                    | "input_token_count"
                    | "output_token_count"
                    | "cached_token_count"
                    | "cache_write_token_count"
                    | "reasoning_token_count"
                    | "duration_ms"
            )
        ) {
            attributes.push(producer.clone());
        }
    }
    if let Some(model) = string_attribute(model, "model") {
        attributes.push(attribute(semconv::GEN_AI_REQUEST_MODEL, model));
    }
    for (target, claude_source, codex_source) in [
        (
            semconv::GEN_AI_INPUT_TOKENS,
            "input_tokens",
            "input_token_count",
        ),
        (
            semconv::GEN_AI_OUTPUT_TOKENS,
            "output_tokens",
            "output_token_count",
        ),
        (
            semconv::GEN_AI_CACHE_READ_INPUT_TOKENS,
            "cache_read_tokens",
            "cached_token_count",
        ),
        (
            semconv::GEN_AI_CACHE_CREATION_INPUT_TOKENS,
            "cache_creation_tokens",
            "cache_write_token_count",
        ),
    ] {
        let source = if claude { claude_source } else { codex_source };
        if let Some(mut tokens) = numeric_attribute(model, source) {
            if claude && target == semconv::GEN_AI_INPUT_TOKENS {
                tokens = tokens
                    .saturating_add(numeric_attribute(model, "cache_read_tokens").unwrap_or(0))
                    .saturating_add(numeric_attribute(model, "cache_creation_tokens").unwrap_or(0));
            }
            attributes.push(json!({"key":target,"value":{"intValue":tokens.to_string()}}));
        }
    }
    attributes
}

fn native_interval(streams: &[NativeStream], session: SessionId, time: u64) -> Option<(u64, u64)> {
    streams
        .iter()
        .filter(|stream| stream.session == session)
        .map(|stream| (stream.start, stream.end))
        // Completion export can follow closure within the producer timestamp's millisecond.
        .filter(|(start, end)| {
            *start <= time && *start < *end && time / 1_000_000 <= end / 1_000_000
        })
        .min_by_key(|(_, end)| end.abs_diff(time))
}

/// Agent model records that can become model spans once their turn is known.
fn model_candidates(source: &TelemetryRecord) -> Vec<PendingModel> {
    let mut candidates = Vec::new();
    for (ordinal, (resource, model)) in signal_records(&source.payload, Signal::Logs).enumerate() {
        let name = string_attribute(model, semconv::EVENT_NAME);
        let claude = model["body"]["stringValue"] == "claude_code.api_request"
            || name == Some("claude_code.api_request");
        let codex = name == Some("codex.sse_event")
            && string_attribute(model, "event.kind") == Some("response.completed");
        if !claude && !codex {
            continue;
        }
        if codex && numeric_attribute(model, "output_token_count").is_none_or(|n| n == 0) {
            continue;
        }
        let Some(session) = string_attribute(&resource["resource"], semconv::SESSION_ID)
            .or_else(|| string_attribute(model, semconv::SESSION_ID))
            .and_then(|s| s.parse::<SessionId>().ok())
        else {
            continue;
        };
        let Some(time) = number(&model["timeUnixNano"])
            .filter(|t| *t > 0)
            .or_else(|| number(&model["observedTimeUnixNano"]))
        else {
            continue;
        };
        let duration_ms = if claude {
            let Some(duration) = numeric_attribute(model, "duration_ms") else {
                continue;
            };
            Some(duration)
        } else {
            None
        };
        candidates.push(PendingModel {
            event_id: source.event_id,
            received_at_ms: source.received_at_ms,
            ordinal,
            session,
            time,
            duration_ms,
            record: model.clone(),
        });
    }
    candidates
}

/// Measured native stream intervals that time Codex completions.
fn native_streams(record: &TelemetryRecord) -> Vec<NativeStream> {
    record.payload["resourceSpans"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let session = string_attribute(&r["resource"], semconv::SESSION_ID)?
                .parse::<SessionId>()
                .ok()?;
            Some((session, r))
        })
        .flat_map(|(session, r)| {
            r["scopeSpans"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|s| s["spans"].as_array().into_iter().flatten())
                .map(move |s| (session, s))
        })
        .filter(|(_, s)| {
            matches!(
                s["name"].as_str(),
                Some(
                    "receiving_stream"
                        | "responses_websocket.stream_request"
                        | "responses.stream_request"
                )
            )
        })
        .filter_map(|(session, s)| {
            Some(NativeStream {
                session,
                start: number(&s["startTimeUnixNano"])?,
                end: number(&s["endTimeUnixNano"])?,
            })
        })
        .collect()
}

fn string_attribute<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value["attributes"]
        .as_array()?
        .iter()
        .find(|a| a["key"] == key)?["value"]["stringValue"]
        .as_str()
}

fn numeric_attribute(value: &Value, key: &str) -> Option<u64> {
    let value = &value["attributes"]
        .as_array()?
        .iter()
        .find(|a| a["key"] == key)?["value"];
    number(&value["intValue"]).or_else(|| number(&value["stringValue"]))
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
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
    async fn s2_claude_model_records_join_closed_turns_with_exact_intervals() {
        for (logs, hooks_file, count) in [
            ("baseline-logs.json", "baseline-hooks.jsonl", 2),
            ("detailed-json-logs.json", "detailed-json-hooks.jsonl", 2),
        ] {
            let sandbox = tempfile::tempdir().unwrap();
            let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
            let session = sessions.create(CreateSession::default()).unwrap();
            let log = TelemetryLog::new(sandbox.path(), session.device_id);
            let store = Arc::new(Mutex::new(sessions));
            replay_hooks(&store, session.id, hooks_file).await;
            drop(store);
            let mut payload: Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{FIXTURES}{logs}")).unwrap(),
            )
            .unwrap();
            replace_session(&mut payload, session.id);
            log.append(Signal::Logs, vec![payload], 0).unwrap();
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            let spans = stored_spans(&log);
            let turn = spans
                .iter()
                .find(|s| s["name"] == semconv::TURN_SPAN)
                .unwrap();
            let models: Vec<_> = spans
                .iter()
                .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
                .collect();
            assert_eq!(models.len(), count, "{logs}");
            for model in models {
                assert_eq!(model["parentSpanId"], turn["spanId"]);
                assert_eq!(model["traceId"], turn["traceId"]);
                assert_eq!(model["kind"], 3);
                assert_eq!(attribute(model, semconv::TIMING), "measured");
                assert_eq!(
                    attribute(model, semconv::GEN_AI_REQUEST_MODEL),
                    "claude-haiku-5-5"
                );
            }
            if logs == "baseline-logs.json" {
                let model = spans
                    .iter()
                    .find(|s| {
                        s["name"] == semconv::MODEL_CALL_SPAN
                            && attribute(s, "request_id") == "synthetic-id-4"
                    })
                    .unwrap();
                assert_eq!(model["startTimeUnixNano"], "1792185033284000000");
                assert_eq!(model["endTimeUnixNano"], "1792185034243000000");
                assert_eq!(
                    attribute_value(model, semconv::GEN_AI_INPUT_TOKENS)["intValue"],
                    "5058"
                );
                assert_eq!(
                    attribute_value(model, semconv::GEN_AI_OUTPUT_TOKENS)["intValue"],
                    "108"
                );
                assert_eq!(
                    attribute_value(model, "cache_read_tokens")["intValue"],
                    2135
                );
            }
            let first = log.read().unwrap();
            drop(Sessions::open(sandbox.path(), 0).unwrap());
            assert_eq!(log.read().unwrap(), first);
        }
    }

    #[tokio::test]
    async fn s2_codex_completions_use_hook_gaps_and_skip_prewarm() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let mut payload: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{FIXTURES}codex-baseline-logs.json")).unwrap(),
        )
        .unwrap();
        replace_session(&mut payload, session.id);
        // Move the captured prewarm inside the turn so time filtering cannot hide it.
        for resource in payload["resourceLogs"].as_array_mut().unwrap() {
            for scope in resource["scopeLogs"].as_array_mut().unwrap() {
                for record in scope["logRecords"].as_array_mut().unwrap() {
                    if record["attributes"].as_array().unwrap().iter().any(|a| {
                        a["key"] == "output_token_count" && a["value"]["stringValue"] == "0"
                    }) {
                        record["observedTimeUnixNano"] = json!("1792185669800000000");
                    }
                }
            }
        }
        log.append(Signal::Logs, vec![payload], 0).unwrap();
        let store = Arc::new(Mutex::new(sessions));
        replay_hooks(&store, session.id, "codex-baseline-hooks.jsonl").await;
        let spans = stored_spans(&log);
        let models: Vec<_> = spans
            .iter()
            .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .collect();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["startTimeUnixNano"], "1792185669774019000");
        assert_eq!(models[0]["endTimeUnixNano"], "1792185671889122000");
        assert_eq!(models[1]["startTimeUnixNano"], "1792185671973413000");
        assert_eq!(models[1]["endTimeUnixNano"], "1792185673297075000");
        for model in models {
            assert_eq!(attribute(model, semconv::TIMING), "estimated");
            assert!(
                !model["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["key"] == "request_id" || a["key"] == "duration_ms")
            );
        }
        assert_eq!(
            attribute_value(
                spans
                    .iter()
                    .find(|s| s["name"] == semconv::MODEL_CALL_SPAN)
                    .unwrap(),
                semconv::GEN_AI_OUTPUT_TOKENS
            )["intValue"],
            "54"
        );
        assert_eq!(
            attribute_value(
                spans
                    .iter()
                    .find(|s| s["name"] == semconv::MODEL_CALL_SPAN)
                    .unwrap(),
                semconv::GEN_AI_INPUT_TOKENS
            )["intValue"],
            "14242"
        );
    }

    #[tokio::test]
    async fn s2_codex_native_streams_supply_measured_model_intervals() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        for (name, signal) in [
            ("codex-baseline-logs.json", Signal::Logs),
            ("codex-baseline-traces.json", Signal::Traces),
        ] {
            let mut payload: Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{FIXTURES}{name}")).unwrap(),
            )
            .unwrap();
            replace_session(&mut payload, session.id);
            log.append(signal, vec![payload], 0).unwrap();
        }
        let store = Arc::new(Mutex::new(sessions));
        replay_hooks(&store, session.id, "codex-baseline-hooks.jsonl").await;
        let spans = stored_spans(&log);
        let models: Vec<_> = spans
            .iter()
            .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .collect();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["startTimeUnixNano"], "1792185669788480000");
        assert_eq!(models[0]["endTimeUnixNano"], "1792185671853870000");
        assert_eq!(models[1]["startTimeUnixNano"], "1792185672011128000");
        assert_eq!(models[1]["endTimeUnixNano"], "1792185673259261000");
        assert!(
            models
                .iter()
                .all(|s| attribute(s, semconv::TIMING) == "measured")
        );
        let first = log.read().unwrap();
        drop(store);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        assert_eq!(log.read().unwrap(), first);
    }

    #[test]
    fn estimated_model_intervals_include_wait_hooks() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        sessions
            .append(session.id, "turn.started", json!({"observed_at_us":1000}))
            .unwrap();
        sessions
            .append(session.id, "agent.waiting", json!({"observed_at_us":3000}))
            .unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        log.append(
            Signal::Logs,
            vec![codex_completion(session.id, 5_000_000)],
            0,
        )
        .unwrap();
        sessions
            .append(session.id, "turn.ended", json!({"observed_at_us":9000}))
            .unwrap();
        let spans = stored_spans(&log);
        let model = spans
            .iter()
            .find(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .unwrap();
        assert_eq!(model["startTimeUnixNano"], "3000000");
        assert_eq!(model["endTimeUnixNano"], "9000000");
    }

    #[test]
    fn model_records_join_only_the_session_and_turn_that_contain_the_completion() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let a = sessions.create(CreateSession::default()).unwrap();
        let b = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), a.device_id);
        for (id, start, end) in [(a.id, 1000, 3000), (a.id, 5000, 9000), (b.id, 1000, 9000)] {
            sessions
                .append(id, "turn.started", json!({"observed_at_us":start}))
                .unwrap();
            sessions
                .append(id, "turn.ended", json!({"observed_at_us":end}))
                .unwrap();
        }
        log.append(
            Signal::Logs,
            vec![
                codex_completion(a.id, 2_000_000),
                codex_completion(a.id, 6_000_000),
                codex_completion(a.id, 4_000_000),
                codex_completion(b.id, 4_000_000),
                codex_completion(Ulid::new(), 2_000_000),
            ],
            0,
        )
        .unwrap();
        drop(sessions);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        let spans = stored_spans(&log);
        let models: Vec<_> = spans
            .iter()
            .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .collect();
        assert_eq!(models.len(), 3);
        let a_trace = format!("{:032x}", u128::from(a.id));
        assert_eq!(models.iter().filter(|s| s["traceId"] == a_trace).count(), 2);
        for model in models {
            let turn = spans
                .iter()
                .find(|s| s["traceId"] == model["traceId"] && s["spanId"] == model["parentSpanId"])
                .unwrap();
            assert_eq!(model["startTimeUnixNano"], turn["startTimeUnixNano"]);
            assert_eq!(model["endTimeUnixNano"], turn["endTimeUnixNano"]);
        }
    }

    #[test]
    fn model_synthesis_scans_only_new_records_and_joins_early_models_later() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let ordinary =
            json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{"timeUnixNano":"1"}]}]}]});
        log.append(Signal::Logs, vec![ordinary; 50], 0).unwrap();
        log.append(
            Signal::Logs,
            vec![codex_completion(session.id, 5_000_000)],
            0,
        )
        .unwrap();
        sessions
            .append(session.id, "turn.started", json!({"observed_at_us":1000}))
            .unwrap();
        let before = sessions.spans.lock().unwrap().scanned;
        assert!(before >= 51);
        sessions
            .append(session.id, "turn.ended", json!({"observed_at_us":9000}))
            .unwrap();
        let scanned = sessions.spans.lock().unwrap().scanned - before;
        assert!(scanned < 50, "rescanned {scanned} records");
        let spans = stored_spans(&log);
        let model = spans
            .iter()
            .find(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .unwrap();
        assert_eq!(model["startTimeUnixNano"], "1000000");
        assert_eq!(model["endTimeUnixNano"], "9000000");
    }

    #[test]
    fn recovery_joins_each_pending_model_a_bounded_number_of_times() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let session = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        // Completions before the first turn never join and stay pending.
        let models = 200;
        log.append(
            Signal::Logs,
            (0..models)
                .map(|_| codex_completion(session.id, 1))
                .collect(),
            0,
        )
        .unwrap();
        let entries = 1_000;
        for i in 0..entries {
            sessions
                .append(
                    session.id,
                    "runner.usage.updated",
                    json!({"observed_at_us": 1000 + i}),
                )
                .unwrap();
        }
        drop(sessions);
        let sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let joins = sessions.spans.lock().unwrap().joins;
        assert!(
            joins <= 2 * models,
            "recovery made {joins} join attempts for {models} models"
        );
    }

    #[test]
    fn an_entry_retries_only_the_pending_models_of_its_own_session() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let idle = sessions.create(CreateSession::default()).unwrap();
        let busy = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(sandbox.path(), idle.device_id);
        log.append(
            Signal::Logs,
            (0..100).map(|_| codex_completion(idle.id, 1)).collect(),
            0,
        )
        .unwrap();
        sessions
            .append(idle.id, "turn.started", json!({"observed_at_us":1000}))
            .unwrap();
        let before = sessions.spans.lock().unwrap().joins;
        for i in 0..100 {
            sessions
                .append(
                    busy.id,
                    "runner.usage.updated",
                    json!({"observed_at_us": 2000 + i}),
                )
                .unwrap();
        }
        let joins = sessions.spans.lock().unwrap().joins - before;
        assert_eq!(
            joins, 0,
            "entries of another session retried {joins} models"
        );
    }

    #[test]
    fn recovered_spans_equal_the_incremental_spans() {
        let live = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(live.path(), 0).unwrap();
        let first = sessions.create(CreateSession::default()).unwrap();
        let second = sessions.create(CreateSession::default()).unwrap();
        let log = TelemetryLog::new(live.path(), first.device_id);
        // Early models join after their turns close. Late models join on the next entry.
        log.append(
            Signal::Logs,
            vec![
                codex_completion(first.id, 1),
                codex_completion(first.id, 5_000_000),
                codex_completion(second.id, 15_000_000),
            ],
            0,
        )
        .unwrap();
        for (session, start) in [(first.id, 1000), (second.id, 12_000), (first.id, 20_000)] {
            sessions
                .append(session, "turn.started", json!({"observed_at_us": start}))
                .unwrap();
            sessions
                .append(
                    session,
                    "tool.pre",
                    json!({"observed_at_us": start + 2000, "tool_use_id": format!("t{start}")}),
                )
                .unwrap();
            sessions
                .append(
                    session,
                    "runner.usage.updated",
                    json!({"observed_at_us": start + 3000}),
                )
                .unwrap();
            sessions
                .append(
                    session,
                    "tool.post",
                    json!({"observed_at_us": start + 4000, "tool_use_id": format!("t{start}")}),
                )
                .unwrap();
            sessions
                .append(
                    session,
                    "turn.ended",
                    json!({"observed_at_us": start + 8000}),
                )
                .unwrap();
        }
        log.append(
            Signal::Logs,
            vec![codex_completion(first.id, 25_000_000)],
            0,
        )
        .unwrap();
        sessions
            .append(first.id, "exit", json!({"observed_at_us":30_000,"code":0}))
            .unwrap();
        drop(sessions);
        let expected = sorted_spans(&log);
        assert_eq!(
            expected
                .iter()
                .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
                .count(),
            3
        );

        let recovered = tempfile::tempdir().unwrap();
        copy_dir(live.path(), recovered.path());
        let path = recovered.path().join("telemetry/log.jsonl");
        let kept: String = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("\"signal\":\"traces\""))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, kept).unwrap();
        drop(Sessions::open(recovered.path(), 0).unwrap());
        let log = TelemetryLog::new(recovered.path(), first.device_id);
        assert_eq!(sorted_spans(&log), expected);
    }

    fn sorted_spans(log: &TelemetryLog) -> Vec<Value> {
        let mut spans = stored_spans(log);
        spans.sort_by_key(|s| (s["traceId"].to_string(), s["spanId"].to_string()));
        spans
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn codex_completion(session: Ulid, time: u64) -> Value {
        json!({"resourceLogs":[{"resource":{"attributes":[{"key":semconv::SESSION_ID,"value":{"stringValue":session.to_string()}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":time.to_string(),"attributes":[{"key":"event.name","value":{"stringValue":"codex.sse_event"}},{"key":"event.kind","value":{"stringValue":"response.completed"}},{"key":"output_token_count","value":{"stringValue":"7"}}]}]}]}]})
    }

    #[test]
    fn missing_native_timing_falls_back_to_an_estimate_when_the_session_exits() {
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
        let log = TelemetryLog::new(sandbox.path(), session.device_id);
        let payload = json!({"resourceLogs":[{"resource":{"attributes":[{"key":semconv::SESSION_ID,"value":{"stringValue":session.id.to_string()}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":"5000000","attributes":[{"key":"event.name","value":{"stringValue":"codex.sse_event"}},{"key":"event.kind","value":{"stringValue":"response.completed"}},{"key":"output_token_count","value":{"stringValue":"7"}}]}]}]}]});
        log.append(Signal::Logs, vec![payload], 0).unwrap();
        sessions
            .append(session.id, "exit", json!({"observed_at_us":10000,"code":0}))
            .unwrap();
        let spans = stored_spans(&log);
        let models: Vec<_> = spans
            .iter()
            .filter(|s| s["name"] == semconv::MODEL_CALL_SPAN)
            .collect();
        assert_eq!(models.len(), 1);
        assert_eq!(attribute(models[0], semconv::TIMING), "estimated");
        assert_eq!(models[0]["startTimeUnixNano"], "1000000");
        assert_eq!(models[0]["endTimeUnixNano"], "9000000");
    }

    #[test]
    fn turn_links_include_target_turns_appended_after_an_earlier_link_read() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let parent = sessions.create(CreateSession::default()).unwrap();
        let child = sessions
            .create(CreateSession {
                parent_session_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        for (id, start) in [
            (parent.id, 100),
            (child.id, 200),
            (parent.id, 300),
            (child.id, 400),
        ] {
            sessions
                .append(id, "turn.started", json!({"observed_at_us":start}))
                .unwrap();
            sessions
                .append(id, "turn.ended", json!({"observed_at_us":start+50}))
                .unwrap();
        }
        let spans = stored_spans(&TelemetryLog::new(sandbox.path(), child.device_id));
        let parent_trace = format!("{:032x}", u128::from(parent.id));
        let child_trace = format!("{:032x}", u128::from(child.id));
        let parent_links = |span: &Value| {
            span["links"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|link| link["traceId"] == parent_trace)
                .count()
        };
        let children: Vec<_> = spans
            .iter()
            .filter(|s| s["traceId"] == child_trace)
            .collect();
        assert_eq!(children.len(), 2);
        assert_eq!(parent_links(children[1]), 2);
    }

    #[test]
    fn turn_segments_link_parent_dependencies_and_previous_segments() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 0).unwrap();
        let parent = sessions.create(CreateSession::default()).unwrap();
        let dependency = sessions.create(CreateSession::default()).unwrap();
        let child = sessions
            .create(CreateSession {
                parent_session_id: Some(parent.id),
                ..Default::default()
            })
            .unwrap();
        for (id, start) in [(parent.id, 100), (parent.id, 300), (dependency.id, 500)] {
            sessions
                .append(id, "turn.started", json!({"observed_at_us":start}))
                .unwrap();
            sessions
                .append(id, "turn.ended", json!({"observed_at_us":start+50}))
                .unwrap();
        }
        sessions
            .append(child.id, "turn.started", json!({"observed_at_us":1000}))
            .unwrap();
        let edge = sessions
            .append_entry(
                child.id,
                "edge.added",
                json!({"target":dependency.id,"kind":"depends_on"}),
            )
            .unwrap();
        sessions
            .append(child.id, "turn.ended", json!({"observed_at_us":2000}))
            .unwrap();
        sessions
            .append(child.id, "edge.removed", json!({"edge_id":edge.event_id}))
            .unwrap();
        sessions
            .append(child.id, "turn.started", json!({"observed_at_us":3000}))
            .unwrap();
        sessions
            .append(child.id, "turn.ended", json!({"observed_at_us":4000}))
            .unwrap();
        let log = TelemetryLog::new(sandbox.path(), child.device_id);
        let spans = stored_spans(&log);
        let child_trace = format!("{:032x}", u128::from(child.id));
        let children: Vec<_> = spans
            .iter()
            .filter(|s| s["traceId"] == child_trace)
            .collect();
        let roots: Vec<_> = spans
            .iter()
            .filter(|s| s["traceId"] != child_trace)
            .collect();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0]["links"].as_array().unwrap().len(), 3);
        assert_eq!(children[1]["links"].as_array().unwrap().len(), 3);
        assert!(children.iter().all(|s| s.get("parentSpanId").is_none()));
        for root in roots {
            assert!(
                children[0]["links"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|link| link["traceId"] == root["traceId"]
                        && link["spanId"] == root["spanId"])
            );
        }
        let dependency_trace = format!("{:032x}", u128::from(dependency.id));
        assert!(
            !children[1]["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|link| link["traceId"] == dependency_trace)
        );
        assert!(
            children[1]["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|link| link["traceId"] == child_trace
                    && link["spanId"] == children[0]["spanId"])
        );
        let first = log.read().unwrap();
        drop(sessions);
        drop(Sessions::open(sandbox.path(), 0).unwrap());
        assert_eq!(log.read().unwrap(), first);
    }

    async fn replay_hooks(store: &Arc<Mutex<Sessions>>, session: Ulid, name: &str) {
        let samples = std::fs::read_to_string(format!("{FIXTURES}{name}")).unwrap();
        for line in samples.lines() {
            let sample: Value = serde_json::from_str(line).unwrap();
            let state = match sample["payload"]["hook_event_name"].as_str().unwrap() {
                "SessionStart" => continue,
                "SessionEnd" => "exit",
                "Stop" => "stop",
                _ => "working",
            };
            hook(
                store,
                session,
                sample["received_unix_ns"].as_u64().unwrap() / 1000,
                state,
                sample["payload"].clone(),
            )
            .await;
        }
    }

    fn replace_session(value: &mut Value, session: Ulid) {
        match value {
            Value::Object(object) => {
                if object.get("key").and_then(Value::as_str) == Some(semconv::SESSION_ID) {
                    object.insert("value".into(), json!({"stringValue": session.to_string()}));
                } else {
                    for child in object.values_mut() {
                        replace_session(child, session);
                    }
                }
            }
            Value::Array(values) => {
                for child in values {
                    replace_session(child, session);
                }
            }
            _ => {}
        }
    }

    fn attribute_value<'a>(span: &'a Value, name: &str) -> &'a Value {
        &span["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["key"] == name)
            .unwrap()["value"]
    }

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
        assert_eq!(
            first.iter().filter(|r| r.signal == Signal::Traces).count(),
            2
        );
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
        assert!(stored_spans(&log).is_empty());
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
        let saved = sandbox.path().join("telemetry/saved.jsonl");
        std::fs::rename(&broken, &saved).unwrap();
        std::fs::create_dir(&broken).unwrap();
        assert!(
            hooks::ingest(store.clone(), callback.clone(), StopDecision::Accept)
                .await
                .is_err()
        );
        std::fs::remove_dir(&broken).unwrap();
        std::fs::rename(&saved, &broken).unwrap();
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
        assert_eq!(
            first.iter().filter(|r| r.signal == Signal::Traces).count(),
            2
        );
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
        let resource = &records
            .iter()
            .find(|r| r.signal == Signal::Traces)
            .unwrap()
            .payload["resourceSpans"][0]["resource"];
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

    #[test]
    fn the_project_resource_falls_back_to_the_workspace_id() {
        use ws_core::{device::Device, log::SessionLog};
        let sandbox = tempfile::tempdir().unwrap();
        let device = Device::init(sandbox.path(), "test-device").unwrap();
        let session = Ulid::new();
        let source = SessionLog::new(sandbox.path(), session, device.device_id);
        source
            .append(
                0,
                "session.created",
                json!({"alias":1,"workspace_id":"workspace"}),
            )
            .unwrap();
        source
            .append(
                0,
                "turn.started",
                json!({"observed_at_us":100,"turn_id":"turn"}),
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
        let records = TelemetryLog::new(sandbox.path(), device.device_id)
            .read()
            .unwrap();
        let resource = &records
            .iter()
            .find(|r| r.signal == Signal::Traces)
            .unwrap()
            .payload["resourceSpans"][0]["resource"];
        assert_eq!(attribute(resource, semconv::PROJECT), "workspace");
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
