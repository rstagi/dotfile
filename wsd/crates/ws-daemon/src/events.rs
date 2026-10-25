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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendEvent {
    #[serde(rename = "type")]
    event_type: String,
    /// A missing or null body means an empty object.
    body: Option<Map<String, Value>>,
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
    let entry = api::call(store, move |sessions| {
        sessions.append_client_event(&id, request)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(entry)))
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
    /// Appends a `loop.*` event. The entry body records the caller under `caller`.
    fn append_client_event(&mut self, id: &str, request: AppendEvent) -> Result<LogEntry> {
        self.record_call(request.parent_session_id, "session.event", Some(id))?;
        let suffix = request.event_type.strip_prefix(CLIENT_EVENT_PREFIX);
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
        let session = self.get_record(id)?;
        let mut body = request.body.unwrap_or_default();
        body.insert("caller".into(), json!(request.parent_session_id));
        self.append_entry(session.id, &request.event_type, Value::Object(body))
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
            let (_, entry) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":event_type}),
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
