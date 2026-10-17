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
    parent_session_id: Option<SessionId>,
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
    Ok(Json(
        wait_for(
            store,
            id,
            condition,
            request.parent_session_id,
            request.timeout,
        )
        .await?,
    ))
}

/// Subscribes and checks under one lock. Later appends wake the wait without polling.
pub(crate) async fn wait_for(
    store: Store,
    input: String,
    condition: Condition,
    caller: Option<SessionId>,
    timeout: Option<u32>,
) -> std::result::Result<WaitResult, ApiError> {
    let first_condition = condition.clone();
    let (id, mut changes, mut shutdown, matched) = api::call(store.clone(), move |sessions| {
        if *sessions.shutdown.borrow() {
            return Err(Error::ShuttingDown);
        }
        sessions.record_call(caller, "session.wait", Some(&input))?;
        let id = sessions.get_record(&input)?.id;
        Ok((
            id,
            sessions.changes.subscribe(),
            sessions.shutdown.subscribe(),
            sessions.wait_match(id, &first_condition)?,
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
            let matched = api::call(store.clone(), move |sessions| {
                sessions.wait_match(id, &condition)
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
        self.get_record(input)
    }
    fn wait_match(&self, id: SessionId, condition: &Condition) -> Result<Option<WaitResult>> {
        let session = self.get_record(&id.to_string())?;
        match condition {
            Condition::State(state) => {
                Ok((session.state == state.to_string()).then_some(WaitResult {
                    session,
                    event: None,
                }))
            }
            Condition::Event(event_type) => {
                Ok(self
                    .registry
                    .latest_event(id, event_type)?
                    .map(|event| WaitResult {
                        session,
                        event: Some(event),
                    }))
            }
        }
    }
}
