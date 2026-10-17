//! Local phase launches through the headless session supervisor.

use std::{collections::BTreeMap, path::PathBuf};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use ws_core::{ids::SessionId, registry::Session};

use crate::{
    api::{ApiError, Store},
    sessions::Error,
    supervisor::{self, CreateHeadless},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnPhase {
    pub parent_session_id: SessionId,
    pub phase: Phase,
    pub repo: PathBuf,
    pub base: String,
    pub caps_required: Vec<String>,
    /// Maximum runtime in seconds.
    pub max_runtime: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub command: Vec<String>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/phases/spawn", post(spawn))
}

async fn spawn(
    State(store): State<Store>,
    Json(request): Json<SpawnPhase>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    if request.base.trim().is_empty() || request.max_runtime == 0 {
        return Err(ApiError(Error::InvalidRequest(
            "Specify a base and a positive maximum runtime.",
        )));
    }
    let metadata = serde_json::to_value(&request).map_err(|error| ApiError(Error::Json(error)))?;
    let session = supervisor::launch(
        store,
        CreateHeadless {
            headless: true,
            command: request.phase.command,
            cwd: request.repo,
            parent_session_id: Some(request.parent_session_id),
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
            stderr: None,
        },
        metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(session)))
}
