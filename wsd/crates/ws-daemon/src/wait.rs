//! Completion output and event-driven session waits.

use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ws_core::{
    ids::{SessionId, Ulid},
    registry::Session,
    state::SessionState,
};

use crate::{
    api::{self, ApiError, SessionResponse, Store},
    sessions::{Error, Result, Sessions},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoneRequest {
    #[serde(default)]
    output: String,
    parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitRequest {
    #[serde(rename = "for")]
    condition: String,
    timeout: Option<u32>,
    #[serde(default)]
    after_seq: u64,
    parent_session_id: Option<SessionId>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitTarget {
    target: String,
    #[serde(default)]
    after_seq: u64,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WaitMode {
    Any,
    #[default]
    All,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiWaitRequest {
    targets: Vec<WaitTarget>,
    #[serde(default)]
    mode: WaitMode,
    #[serde(rename = "for")]
    condition: String,
    timeout: Option<u32>,
    parent_session_id: Option<SessionId>,
}

#[derive(Serialize)]
struct MultiWaitResult {
    matches: Vec<WaitResult>,
}

#[derive(Clone, Debug)]
pub enum Condition {
    State(SessionState),
    Event(String),
}

#[derive(Debug, Serialize)]
pub struct WaitResult {
    pub session: Session,
    pub event: Option<ws_core::log::LogEntry>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/sessions/{id}/done", post(done))
        .route("/v1/sessions/{id}/wait", post(wait))
        .route("/v1/wait", post(wait_many))
}

async fn done(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<DoneRequest>,
) -> std::result::Result<Json<SessionResponse>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.done(&id, &request.output, request.parent_session_id)
        })
        .await?
        .into(),
    ))
}

async fn wait(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<WaitRequest>,
) -> std::result::Result<Json<WaitResult>, ApiError> {
    let condition = Condition::parse(&request.condition).map_err(ApiError)?;
    let mut result = wait_for(
        store,
        vec![WaitTarget {
            target: id,
            after_seq: request.after_seq,
        }],
        WaitMode::All,
        condition,
        request.parent_session_id,
        request.timeout,
    )
    .await?;
    Ok(Json(result.matches.remove(0)))
}

async fn wait_many(
    State(store): State<Store>,
    Json(request): Json<MultiWaitRequest>,
) -> std::result::Result<Json<MultiWaitResult>, ApiError> {
    let condition = Condition::parse(&request.condition).map_err(ApiError)?;
    Ok(Json(
        wait_for(
            store,
            request.targets,
            request.mode,
            condition,
            request.parent_session_id,
            request.timeout,
        )
        .await?,
    ))
}

/// Subscribes and checks every target under one lock. Appends wake the wait without polling.
async fn wait_for(
    store: Store,
    targets: Vec<WaitTarget>,
    mode: WaitMode,
    condition: Condition,
    caller: Option<SessionId>,
    timeout: Option<u32>,
) -> std::result::Result<MultiWaitResult, ApiError> {
    let first_condition = condition.clone();
    let (targets, mut changes, mut shutdown, matched) = api::call(store.clone(), move |sessions| {
        if *sessions.shutdown.borrow() {
            return Err(Error::ShuttingDown);
        }
        if targets.is_empty() {
            return Err(Error::InvalidRequest("Supply at least one wait target."));
        }
        let mut resolved = Vec::with_capacity(targets.len());
        for target in targets {
            sessions.record_call(caller, "session.wait", Some(&target.target))?;
            let id = sessions.get_record(&target.target)?.id;
            if resolved.iter().any(|(previous, _)| *previous == id) {
                return Err(Error::InvalidRequest(
                    "Use each session only once in a wait.",
                ));
            }
            resolved.push((id, target.after_seq));
        }
        let matched = sessions.wait_matches(&resolved, &first_condition, mode)?;
        Ok((
            resolved,
            sessions.changes.subscribe(),
            sessions.shutdown.subscribe(),
            matched,
        ))
    })
    .await?;
    if let Some(result) = matched {
        return Ok(result);
    }
    let pending = async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => return Err(ApiError(Error::ShuttingDown)),
                changed = changes.changed() => changed.map_err(|_| ApiError(Error::ShuttingDown))?,
            }
            let condition = condition.clone();
            let targets = targets.clone();
            let matched = api::call(store.clone(), move |sessions| {
                sessions.wait_matches(&targets, &condition, mode)
            })
            .await?;
            if let Some(result) = matched {
                return Ok(result);
            }
        }
    };
    match timeout {
        Some(seconds) => {
            tokio::time::timeout(std::time::Duration::from_secs(seconds.into()), pending)
                .await
                .map_err(|_| ApiError(Error::WaitTimeout))?
        }
        None => pending.await,
    }
}

impl Condition {
    pub fn parse(input: &str) -> Result<Self> {
        if let Some(state) = input.strip_prefix("state=") {
            return serde_json::from_value(json!(state))
                .map(Self::State)
                .map_err(|_| Error::InvalidRequest("Use a valid session state after state=."));
        }
        let event = match input {
            "done" | "exit" => input,
            _ => input
                .strip_prefix("event=")
                .filter(|event| !event.is_empty())
                .ok_or(Error::InvalidRequest(
                    "Use done, exit, state=<state> or event=<type> for the wait condition.",
                ))?,
        };
        Ok(Self::Event(event.into()))
    }
}

impl Sessions {
    pub fn done(
        &mut self,
        input: &str,
        output: &str,
        caller: Option<SessionId>,
    ) -> Result<Session> {
        self.record_call(caller, "session.done", Some(input))?;
        let session = self.get_record(input)?;
        let from: SessionState =
            serde_json::from_value(json!(session.state)).map_err(Error::Json)?;
        if from != SessionState::Finished {
            from.transition(SessionState::Finished)
                .map_err(Error::Transition)?;
        }
        if self
            .registry
            .messages()?
            .iter()
            .any(|message| message.recipient == session.id && message.read_at_ms.is_none())
        {
            return Err(Error::PendingMessages);
        }
        let directory = self.state.join("sessions").join(session.id.to_string());
        let path = directory.join("output.md");
        let temporary = directory.join(format!(".output-{}", Ulid::new()));
        let published = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(output.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if let Err(error) = published {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        self.append(
            session.id,
            "done",
            json!({"output":path, "parent_session_id":caller}),
        )?;
        if session.body["spawned"] == true {
            self.finish_spawned_session(&session)?;
        }
        self.get_record(input)
    }
    fn wait_matches(
        &self,
        targets: &[(SessionId, u64)],
        condition: &Condition,
        mode: WaitMode,
    ) -> Result<Option<MultiWaitResult>> {
        let mut matches = Vec::new();
        for &(id, after_seq) in targets {
            if let Some(result) = self.wait_match(id, condition, after_seq)? {
                matches.push(result);
            }
        }
        let satisfied = match mode {
            WaitMode::Any => !matches.is_empty(),
            WaitMode::All => matches.len() == targets.len(),
        };
        Ok(satisfied.then_some(MultiWaitResult { matches }))
    }

    fn wait_match(
        &self,
        id: SessionId,
        condition: &Condition,
        after_seq: u64,
    ) -> Result<Option<WaitResult>> {
        let session = self.get_record(&id.to_string())?;
        match condition {
            Condition::State(state) => {
                let fresh = after_seq == 0
                    || self
                        .registry
                        .latest_event(id, state.event_type())?
                        .is_some_and(|event| event.seq > after_seq)
                    || (*state == SessionState::Finished
                        && self
                            .registry
                            .latest_event(id, "done")?
                            .is_some_and(|event| event.seq > after_seq));
                Ok(
                    (fresh && session.state == state.to_string()).then_some(WaitResult {
                        session,
                        event: None,
                    }),
                )
            }
            Condition::Event(event_type) => {
                let reopened = self.registry.latest_event(id, "session.reopened")?;
                Ok(self
                    .registry
                    .latest_event(id, event_type)?
                    .filter(|event| {
                        event.seq > after_seq
                            && reopened
                                .as_ref()
                                .is_none_or(|reopened| event.seq > reopened.seq)
                    })
                    .map(|event| WaitResult {
                        session,
                        event: Some(event),
                    }))
            }
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

    #[tokio::test]
    async fn multiwait_requires_events_after_each_targets_sequence() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(crate::sessions::Sessions::open(sandbox.path(), 1).unwrap());
        let (_, first) = request(&app, "/v1/sessions", json!({})).await;
        let (_, second) = request(&app, "/v1/sessions", json!({})).await;
        let first = first["id"].as_str().unwrap();
        let second = second["id"].as_str().unwrap();
        let (_, old) = request(
            &app,
            &format!("/v1/sessions/{first}/events"),
            json!({"type":"loop.note"}),
        )
        .await;
        let (_, ready) = request(
            &app,
            &format!("/v1/sessions/{second}/events"),
            json!({"type":"loop.note"}),
        )
        .await;
        let targets =
            json!([{"target":first,"after_seq":old["seq"]},{"target":second,"after_seq":0}]);
        let body = json!({"targets":targets,"mode":"all","for":"event=loop.note","timeout":0});
        let (status, _) = request(&app, "/v1/wait", body.clone()).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        let mut later = body.clone();
        later["targets"][0]["after_seq"] = json!(old["seq"].as_u64().unwrap() + 1);
        let (status, _) = request(&app, "/v1/wait", later).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        let mut any = body.clone();
        any["mode"] = json!("any");
        let (status, result) = request(&app, "/v1/wait", any).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert_eq!(result["matches"][0]["event"], ready);
        let (status, _) = request(
            &app,
            &format!("/v1/sessions/{first}/wait"),
            json!({"for":"event=loop.note","after_seq":old["seq"],"timeout":0}),
        )
        .await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        let (_, new) = request(
            &app,
            &format!("/v1/sessions/{first}/events"),
            json!({"type":"loop.note"}),
        )
        .await;
        let (status, result) = request(&app, "/v1/wait", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["matches"].as_array().unwrap().len(), 2);
        assert_eq!(result["matches"][0]["event"], new);
        assert_eq!(result["matches"][1]["event"], ready);
        let (status, _) = request(
            &app,
            "/v1/wait",
            json!({"targets":[],"mode":"all","for":"done","timeout":0}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    async fn request(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}
