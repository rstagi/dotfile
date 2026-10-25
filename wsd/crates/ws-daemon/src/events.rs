//! Typed client events and ordered log reads for Loop sessions.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ws_core::{ids::SessionId, log::LogEntry};

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};

/// A client may append only event types with this prefix.
pub const CLIENT_EVENT_PREFIX: &str = "loop.";

/// Who produced an event. The daemon sets `daemon` itself. A client may claim `tool` or `agent`.
pub const SOURCE_DAEMON: &str = "daemon";
pub const SOURCE_TOOL: &str = "tool";
pub const SOURCE_AGENT: &str = "agent";

/// The entry body field that holds the key of an append.
const KEY_FIELD: &str = "event_key";
const MAX_KEY_BYTES: usize = 256;

const PHASE_KINDS: [&str; 2] = ["work", "review"];
/// The D19 output kinds of a work phase.
const OUTPUT_KINDS: [&str; 4] = ["pr", "file", "document", "output.md"];
const PHASE_STATUSES: [&str; 5] = ["todo", "claimed", "running", "done", "merged"];

const REVIEW_ROLES: [&str; 7] = [
    "review", "critic", "respond", "confirm", "disputed", "fix", "final",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendEvent {
    #[serde(rename = "type")]
    event_type: String,
    /// A missing or null body means an empty object.
    body: Option<Map<String, Value>>,
    /// `tool` or `agent`. A missing value means `agent`.
    source: Option<String>,
    /// A stable key for one fact. A second append with the same key adds nothing.
    key: Option<String>,
    parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    /// Keep entries whose type starts with this prefix.
    #[serde(rename = "type", default)]
    prefix: String,
    /// Keep entries with a sequence number above this value.
    #[serde(default)]
    since: u64,
    parent_session_id: Option<SessionId>,
}

#[derive(Serialize)]
struct LogResponse {
    session_id: SessionId,
    entries: Vec<LogEntry>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/sessions/{id}/events", axum::routing::post(append))
        .route("/v1/sessions/{id}/log", get(log))
}

async fn append(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<AppendEvent>,
) -> std::result::Result<(StatusCode, Json<LogEntry>), ApiError> {
    let (entry, created) = api::call(store, move |sessions| {
        sessions.append_client_event(&id, request)
    })
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(entry)))
}

async fn log(
    State(store): State<Store>,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> std::result::Result<Json<LogResponse>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.read_log(&id, &query)).await?,
    ))
}

impl Sessions {
    /// Appends a `loop.*` event for a client. The entry body records the caller under `caller`
    /// and the producer under `source`. With a key, a session that already has an event with that
    /// key gets no new entry: the result holds the existing entry and `false`. The check and the
    /// append share one store operation, and the key lives in the durable log.
    fn append_client_event(
        &mut self,
        id: &str,
        request: AppendEvent,
    ) -> Result<(LogEntry, bool)> {
        self.record_call(request.parent_session_id, "session.event", Some(id))?;
        if request.event_type == "loop.control"
            && request
                .body
                .as_ref()
                .is_some_and(|body| body.get("action").is_some_and(|action| action == "ack"))
        {
            return Err(Error::InvalidRequest(
                "Use loop ack to acknowledge a violation.",
            ));
        }
        let source = match request.source.as_deref() {
            None | Some(SOURCE_AGENT) => SOURCE_AGENT,
            Some(SOURCE_TOOL) => SOURCE_TOOL,
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "Use tool or agent as the event source.",
                ));
            }
        };
        let body = request.body.unwrap_or_default();
        if body.contains_key(KEY_FIELD) {
            return Err(Error::InvalidRequest("Use the key field to set a key."));
        }
        let key = request.key;
        if let Some(key) = &key {
            if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
                return Err(Error::InvalidRequest(
                    "Use a key of 1 to 256 characters with no control characters.",
                ));
            }
            // A bad body is refused before the key lookup, so a retry never hides an error.
            check_body(&request.event_type, &body).map_err(Error::InvalidBody)?;
            if let Some(existing) = self.find_keyed_entry(id, key)? {
                return Ok((existing, false));
            }
        }
        let entry = self.append_loop_entry(
            id,
            &request.event_type,
            body,
            source,
            request.parent_session_id,
            key.as_deref(),
        )?;
        Ok((entry, true))
    }

    fn find_keyed_entry(&mut self, id: &str, key: &str) -> Result<Option<LogEntry>> {
        let session = self.get_record(id)?;
        let log = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id).read()?;
        Ok(log.entries.into_iter().find(|entry| {
            entry.event_type.starts_with(CLIENT_EVENT_PREFIX)
                && entry.body.get(KEY_FIELD).is_some_and(|value| value == key)
        }))
    }

    /// Appends a `loop.*` event that the daemon produces itself, with `source: daemon`.
    pub fn append_daemon_loop_event(
        &mut self,
        id: &str,
        event_type: &str,
        body: Map<String, Value>,
    ) -> Result<LogEntry> {
        self.append_loop_entry(id, event_type, body, SOURCE_DAEMON, None, None)
    }

    fn append_loop_entry(
        &mut self,
        id: &str,
        event_type: &str,
        mut body: Map<String, Value>,
        source: &str,
        caller: Option<SessionId>,
        key: Option<&str>,
    ) -> Result<LogEntry> {
        let suffix = event_type.strip_prefix(CLIENT_EVENT_PREFIX);
        if suffix.is_none_or(|suffix| {
            suffix.is_empty()
                || !suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        }) {
            return Err(Error::InvalidRequest(
                "Use an event type that starts with loop. and holds letters, digits, dots, underscores or hyphens.",
            ));
        }
        check_body(event_type, &body).map_err(Error::InvalidBody)?;
        let session = self.get_record(id)?;
        body.insert("caller".into(), json!(caller));
        body.insert("source".into(), json!(source));
        if let Some(key) = key {
            body.insert(KEY_FIELD.into(), json!(key));
        }
        let entry = self.append_entry(session.id, event_type, Value::Object(body))?;
        // The fact and the status that it causes share the serialized store operation.
        // Keep recovery enabled if the second durable append fails.
        if let Err(error) = self.derive_phase_status(session.id, &entry) {
            self.needs_rebuild = true;
            return Err(error);
        }
        Ok(entry)
    }

    fn read_log(&mut self, id: &str, query: &LogQuery) -> Result<LogResponse> {
        self.record_call(query.parent_session_id, "session.log", Some(id))?;
        let session = self.get_record(id)?;
        let log = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id).read()?;
        let entries = log
            .entries
            .into_iter()
            .filter(|entry| entry.seq > query.since && entry.event_type.starts_with(&query.prefix))
            .collect();
        Ok(LogResponse {
            session_id: session.id,
            entries,
        })
    }
}

/// Checks the body of a `loop.*` event against its type.
pub(crate) fn check_body(
    event_type: &str,
    body: &Map<String, Value>,
) -> std::result::Result<(), String> {
    let kind = event_type
        .strip_prefix(CLIENT_EVENT_PREFIX)
        .unwrap_or(event_type);
    let check = Check { event_type, body };
    match kind {
        "plan" => {
            check.string("text")?;
            check.string("sha")
        }
        "phases" => check_phases(&check),
        "phase" => {
            check.integer("n")?;
            check.string("status")?;
            check.optional_integer("attempt")?;
            check.nullable_string("session")
        }
        "attempt" => check_attempt(&check),
        "merge" => {
            check.integer("n")?;
            check.string("repo")?;
            check.string("head")?;
            check.one_of("verify", &["pass", "fail", "none"]).map(drop)
        }
        "milestone" => check.integer("milestone"),
        "gate" => check_gate(&check),
        "review" => check_review(&check),
        "note" => {
            check.string("text")?;
            check.optional_integer("phase")
        }
        "decision" => {
            check.string("topic")?;
            check.present("default")?;
            check.present("choice")?;
            check.string("reason")
        }
        "control" => check.optional_integer("phase"),
        "guard" => {
            check.string("session")?;
            check.string("rule")?;
            check.string("command")
        }
        "violation" => {
            check.string("id")?;
            check.string("rule")?;
            check.one_of("severity", &["hard", "soft"])?;
            check.nullable_integer("phase")?;
            check.present("evidence")
        }
        "finish" => Ok(()),
        _ => Err(format!("{event_type} is not a known loop event type.")),
    }
}

fn check_attempt(check: &Check) -> std::result::Result<(), String> {
    check.integer("n")?;
    check.integer("attempt")?;
    check.string("session")?;
    let state = check.one_of("state", &["started", "exited", "result"])?;
    match state {
        "started" => check.absent(&["exit", "outcome", "verify"]),
        "exited" => {
            check.integer("exit")?;
            check.absent(&["outcome", "verify"])
        }
        _ => {
            check.string("outcome")?;
            check.one_of("verify", &["pass", "fail", "none"])?;
            check.absent(&["exit"])
        }
    }
}

fn check_review(check: &Check) -> std::result::Result<(), String> {
    check.integer("phase")?;
    check.string("repo")?;
    check.string("key")?;
    let role = check.one_of("role", &REVIEW_ROLES)?;
    check.integer("round")?;
    let state = check.one_of("state", &["started", "exited", "finished"])?;
    check.optional_string("commentUrl")?;
    check.optional_string("sidecar")?;
    if role == "disputed" {
        // A disputed round has no runner.
        check.null("session")?;
        check.null("provider")?;
    } else {
        check.string("session")?;
        if state == "finished" {
            check.string("provider")?;
        } else {
            check.optional_string("provider")?;
        }
    }
    if state == "finished" {
        check.string("verdict")
    } else {
        check.absent(&["verdict"])
    }
}

fn check_phases(check: &Check) -> std::result::Result<(), String> {
    check.array_of_objects("phases")?;
    for phase in check.body["phases"].as_array().into_iter().flatten() {
        let phase = Check {
            event_type: check.event_type,
            body: phase.as_object().unwrap(),
        };
        phase.integer("n")?;
        phase.string("title")?;
        phase.one_of("kind", &PHASE_KINDS)?;
        phase.one_of("output", &OUTPUT_KINDS)?;
        phase.integer("milestone")?;
        phase.nullable_one_of("gate", &["merge", "accept"])?;
        phase.nullable_string("repo")?;
        phase.nullable_string("branch")?;
        phase.array_of_integers("depends_on")?;
        phase.one_of("status", &PHASE_STATUSES)?;
    }
    Ok(())
}

fn check_gate(check: &Check) -> std::result::Result<(), String> {
    let kind = check.one_of("kind", &["merge", "accept"])?;
    check.integer("milestone")?;
    if kind == "merge" {
        check.one_of("state", &["open", "merged"])?;
    } else {
        check.one_of("state", &["open", "accepted", "rejected"])?;
        check.integer("phase")?;
    }
    let field = if kind == "merge" { "prs" } else { "artifacts" };
    match check.body.get(field) {
        Some(Value::Array(_) | Value::Object(_)) => Ok(()),
        _ => Err(format!(
            "{} body needs `{field}` as an array or an object for the gate kind {kind}.",
            check.event_type
        )),
    }
}

/// Field checks that name the event type in each error.
struct Check<'a> {
    event_type: &'a str,
    body: &'a Map<String, Value>,
}

impl Check<'_> {
    fn fail(&self, key: &str, expected: &str) -> String {
        format!("{} body needs `{key}` as {expected}.", self.event_type)
    }

    fn string(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(()),
            _ => Err(self.fail(key, "a non-empty string")),
        }
    }

    fn optional_string(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            None | Some(Value::Null | Value::String(_)) => Ok(()),
            _ => Err(self.fail(key, "a string")),
        }
    }

    /// The key must be present. Its value is a non-empty string or null.
    fn nullable_string(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::Null) => Ok(()),
            Some(_) => self.string(key).map(drop),
            None => Err(self.fail(key, "a string or null")),
        }
    }

    fn null(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            None | Some(Value::Null) => Ok(()),
            _ => Err(format!(
                "{} body has `{key}` null for this role.",
                self.event_type
            )),
        }
    }

    fn integer(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(value) if value.is_i64() || value.is_u64() => Ok(()),
            _ => Err(self.fail(key, "an integer")),
        }
    }

    fn optional_integer(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            None | Some(Value::Null) => Ok(()),
            _ => self.integer(key),
        }
    }

    fn nullable_integer(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::Null) => Ok(()),
            _ => self.integer(key),
        }
    }

    fn present(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            None | Some(Value::Null) => Err(self.fail(key, "a value")),
            Some(_) => Ok(()),
        }
    }

    fn one_of(&self, key: &str, values: &[&str]) -> std::result::Result<&str, String> {
        match self.body.get(key).and_then(Value::as_str) {
            Some(value) if values.contains(&value) => Ok(value),
            _ => Err(self.fail(key, &format!("one of {}", values.join(", ")))),
        }
    }

    fn nullable_one_of(&self, key: &str, values: &[&str]) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::Null) => Ok(()),
            _ => self.one_of(key, values).map(drop),
        }
    }

    fn array_of_integers(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::Array(items)) if items.iter().all(|i| i.is_i64() || i.is_u64()) => Ok(()),
            _ => Err(self.fail(key, "an array of integers")),
        }
    }

    fn array_of_objects(&self, key: &str) -> std::result::Result<(), String> {
        match self.body.get(key) {
            Some(Value::Array(items)) if items.iter().all(Value::is_object) => Ok(()),
            _ => Err(self.fail(key, "an array of objects")),
        }
    }

    fn absent(&self, keys: &[&str]) -> std::result::Result<(), String> {
        match keys
            .iter()
            .find(|key| self.body.get(**key).is_some_and(|value| !value.is_null()))
        {
            Some(key) => Err(format!(
                "{} body must not have `{key}` for this state.",
                self.event_type
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::sessions::Sessions;

    #[tokio::test]
    async fn accepted_append_records_the_event_and_its_caller() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, loop_session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, caller) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = loop_session["id"].as_str().unwrap();
        let (status, entry) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/events"),
            json!({"type":"loop.note","body":{"text":"hi"},"parent_session_id":caller["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(entry["type"], "loop.note");
        assert_eq!(entry["body"]["text"], "hi");
        assert_eq!(entry["body"]["caller"], caller["id"]);
        let (_, log) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}/log?type=loop."),
            Value::Null,
        )
        .await;
        assert_eq!(log["entries"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn append_rejects_every_type_outside_the_loop_prefix() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        for event_type in [
            "session.done",
            "api.called",
            "loop",
            "loop.",
            "xloop.note",
            "loop.a b",
        ] {
            let (status, _) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":event_type}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{event_type}");
        }
        let (_, log) = request(&app, "GET", &format!("/v1/sessions/{id}/log"), Value::Null).await;
        assert!(
            log["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| !entry["type"].as_str().unwrap().starts_with("loop"))
        );
    }

    #[tokio::test]
    async fn log_filters_by_type_prefix_and_sequence_in_order() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let mut seqs = Vec::new();
        for event_type in ["loop.phase", "loop.note", "loop.phase"] {
            let body = if event_type == "loop.note" {
                json!({"text":"hi"})
            } else {
                json!({"n":1,"status":"todo","attempt":1,"session":null})
            };
            let (_, entry) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":event_type,"body":body}),
            )
            .await;
            seqs.push((event_type, entry["seq"].as_u64().unwrap()));
        }
        let (status, log) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}/log?type=loop.phase"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let found: Vec<_> = log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(found, [seqs[0].1, seqs[2].1]);
        let (_, log) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}/log?type=loop.&since={}", seqs[0].1),
            Value::Null,
        )
        .await;
        let found: Vec<_> = log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["seq"].as_u64().unwrap())
            .collect();
        assert_eq!(found, [seqs[1].1, seqs[2].1]);
        let (status, _) = request(&app, "GET", "/v1/sessions/missing/log", Value::Null).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    fn valid_bodies() -> Vec<(&'static str, Value)> {
        vec![
            ("loop.plan", json!({"text":"# Plan","sha":"abc"})),
            (
                "loop.phases",
                json!({"phases":[{"n":1,"title":"A","kind":"work","output":"pr","milestone":1,
                    "gate":null,"repo":null,"branch":null,"depends_on":[],"status":"todo"}]}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"in-progress","attempt":1,"session":"01ARZ3NDEKTSV4RRFFQ69G5FAA"}),
            ),
            (
                "loop.attempt",
                json!({"n":1,"attempt":1,"session":"s","state":"started"}),
            ),
            (
                "loop.attempt",
                json!({"n":1,"attempt":1,"session":"s","state":"exited","exit":0}),
            ),
            (
                "loop.attempt",
                json!({"n":1,"attempt":1,"session":"s","state":"result","outcome":"done","verify":"pass"}),
            ),
            (
                "loop.merge",
                json!({"n":1,"repo":"o/r","head":"abc123","verify":"pass"}),
            ),
            ("loop.milestone", json!({"milestone":5})),
            (
                "loop.gate",
                json!({"kind":"merge","state":"open","milestone":1,"prs":{"o/r":"https://example.test/pull/1"}}),
            ),
            (
                "loop.gate",
                json!({"kind":"accept","state":"open","milestone":1,"phase":4,"artifacts":["out.md"]}),
            ),
            (
                "loop.review",
                json!({"phase":146,"repo":"o/r","key":"k","role":"review","round":1,"state":"started","session":"s"}),
            ),
            (
                "loop.review",
                json!({"phase":146,"repo":"o/r","key":"k","role":"final","round":2,"state":"finished","session":"s","provider":"claude","verdict":"APPROVE","commentUrl":"https://example.test/c"}),
            ),
            (
                "loop.review",
                json!({"phase":146,"repo":"o/r","key":"k","role":"disputed","round":1,"state":"finished","session":null,"provider":null,"verdict":"DISPUTED","sidecar":"r.disputed.md"}),
            ),
            ("loop.note", json!({"text":"hi","phase":3})),
            (
                "loop.decision",
                json!({"topic":"rounds","default":2,"choice":1,"reason":"small diff"}),
            ),
            ("loop.control", json!({"action":"pause"})),
            (
                "loop.guard",
                json!({"session":"s","rule":"no-push","command":"git push"}),
            ),
            (
                "loop.violation",
                json!({"id":"v1","rule":"r","severity":"hard","phase":null,"evidence":{"seq":3}}),
            ),
            ("loop.finish", json!({})),
        ]
    }

    #[tokio::test]
    async fn every_event_type_accepts_a_valid_body() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        for (event_type, body) in valid_bodies() {
            let (status, entry) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":event_type,"body":body}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{event_type} {body}: {entry}");
            assert_eq!(entry["body"]["source"], "agent");
        }
    }

    #[tokio::test]
    async fn a_body_that_does_not_match_its_type_is_rejected_with_a_reason() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let attempt = json!({"n":1,"attempt":1,"session":"s"});
        let review =
            json!({"phase":1,"repo":"o/r","key":"k","role":"review","round":1,"session":"s"});
        let merge = |patch: Value| {
            let mut body = attempt.clone();
            body.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            body
        };
        let review_with = |patch: Value| {
            let mut body = review.clone();
            body.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            body
        };
        let cases = [
            ("loop.phase", json!({})),
            ("loop.plan", json!({"text":"x"})),
            ("loop.note", json!({"text":7})),
            (
                "loop.merge",
                json!({"n":1,"repo":"o/r","head":"h","verify":"maybe"}),
            ),
            ("loop.unknown", json!({})),
            // loop.attempt depends on its state.
            ("loop.attempt", merge(json!({"state":"paused"}))),
            ("loop.attempt", merge(json!({"state":"started","exit":0}))),
            ("loop.attempt", merge(json!({"state":"exited"}))),
            (
                "loop.attempt",
                merge(json!({"state":"exited","exit":1,"verify":"pass"})),
            ),
            (
                "loop.attempt",
                merge(json!({"state":"result","outcome":"done"})),
            ),
            (
                "loop.attempt",
                merge(json!({"state":"result","outcome":"done","verify":"yes"})),
            ),
            // loop.review depends on its state and role.
            (
                "loop.review",
                review_with(json!({"state":"started","verdict":"APPROVE"})),
            ),
            (
                "loop.review",
                review_with(json!({"state":"finished","verdict":"APPROVE"})),
            ),
            (
                "loop.review",
                review_with(json!({"state":"finished","provider":"claude"})),
            ),
            (
                "loop.review",
                review_with(json!({"state":"started","role":"judge"})),
            ),
            (
                "loop.review",
                review_with(json!({"state":"started","role":"disputed"})),
            ),
            ("loop.gate", json!({"kind":"merge","state":"open"})),
        ];
        for (event_type, body) in cases {
            let (status, error) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":event_type,"body":body}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{event_type} {body}");
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(event_type),
                "{error}"
            );
        }
        let (_, log) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}/log?type=loop."),
            Value::Null,
        )
        .await;
        assert!(log["entries"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_source_is_agent_by_default_tool_on_request_and_daemon_for_the_daemon() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let id = sessions.create(Default::default()).unwrap().id.to_string();
        let note = || serde_json::from_value(json!({"text":"n"})).unwrap();
        let entry = sessions
            .append_daemon_loop_event(&id, "loop.note", note())
            .unwrap();
        assert_eq!(entry.body["source"], "daemon");
        assert_eq!(entry.body["caller"], Value::Null);
        let invalid = sessions.append_daemon_loop_event(&id, "loop.note", Default::default());
        assert!(invalid.is_err());

        let app = super::super::api::router(sessions);
        for (source, expected) in [
            (json!(null), "agent"),
            (json!("agent"), "agent"),
            (json!("tool"), "tool"),
        ] {
            let (status, entry) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":"loop.note","body":{"text":"n","source":"daemon"},"source":source}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(
                entry["body"]["source"], expected,
                "a body cannot set the source"
            );
        }
        let (status, _) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/events"),
            json!({"type":"loop.note","body":{"text":"n"},"source":"daemon"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_makes_one_loop_session_with_no_process_per_directory() {
        let sandbox = tempfile::tempdir().unwrap();
        let loop_dir = tempfile::tempdir().unwrap();
        let app = super::super::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, created) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":loop_dir.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["body"]["kind"], "loop");
        let (status, again) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":loop_dir.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["id"], created["id"]);
        let (status, _) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":"relative"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let id = created["id"].as_str().unwrap();
        let (_, log) = request(&app, "GET", &format!("/v1/sessions/{id}/log"), Value::Null).await;
        assert!(log["entries"].as_array().unwrap().iter().all(|entry| {
            !entry["type"]
                .as_str()
                .unwrap()
                .starts_with("session.running")
        }));
    }

    #[test]
    fn a_loop_session_and_its_events_survive_a_restart() {
        let sandbox = tempfile::tempdir().unwrap();
        let loop_dir = tempfile::tempdir().unwrap();
        let note = || serde_json::from_value(json!({"text":"n"})).unwrap();
        let id = {
            let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
            let (created, session) = sessions.create_loop_session(loop_dir.path(), None).unwrap();
            assert!(created);
            let id = session.id.to_string();
            sessions
                .append_daemon_loop_event(&id, "loop.note", note())
                .unwrap();
            id
        };
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let (created, session) = sessions.create_loop_session(loop_dir.path(), None).unwrap();
        assert!(!created);
        assert_eq!(session.id.to_string(), id);
        let entry = sessions
            .append_daemon_loop_event(&id, "loop.note", note())
            .unwrap();
        assert_eq!(entry.body["source"], "daemon");
    }

    async fn request(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(if body.is_null() {
                        Body::empty()
                    } else {
                        Body::from(body.to_string())
                    })
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}
