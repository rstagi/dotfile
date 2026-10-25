//! Loop controls use the existing shell orchestrator and dashboard control files.
//! A loop session holds the Loop state as events. It has no process.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ws_core::{ids::SessionId, registry::Session};

use crate::{
    api::{self, ApiError, SessionResponse, Store},
    sessions::{Error, Result, Sessions},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopRequest {
    pub loop_dir: PathBuf,
    pub parent_session_id: Option<SessionId>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/loops/create", post(create))
        .route("/v1/loops/start", post(start))
        .route("/v1/loops/stop", post(stop))
}

/// Resolves an installed Loop script. An environment override supports custom installations.
pub fn script_path(variable: &str, filename: &str) -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| std::io::Error::other("Set HOME to locate the Loop scripts."))?;
    Ok(PathBuf::from(home).join("dotfile").join(filename))
}

/// Creates the session of kind `loop` for a loop directory, or returns the one that exists.
async fn create(
    State(store): State<Store>,
    Json(request): Json<LoopRequest>,
) -> std::result::Result<(axum::http::StatusCode, Json<SessionResponse>), ApiError> {
    let (created, session) = api::call(store, move |sessions| {
        sessions.record_call(
            request.parent_session_id,
            "loop.create",
            request.loop_dir.to_str(),
        )?;
        sessions.create_loop_session(&request.loop_dir, request.parent_session_id)
    })
    .await?;
    let status = if created {
        axum::http::StatusCode::CREATED
    } else {
        axum::http::StatusCode::OK
    };
    Ok((status, Json(session.into())))
}

impl Sessions {
    /// Returns `(true, session)` for a new loop session and `(false, session)` for the session
    /// that already holds this canonical loop directory.
    pub fn create_loop_session(
        &mut self,
        loop_dir: &Path,
        parent_session_id: Option<SessionId>,
    ) -> Result<(bool, Session)> {
        if !loop_dir.is_absolute() {
            return Err(Error::InvalidRequest(
                "The loop directory must be absolute.",
            ));
        }
        let directory = loop_dir.canonicalize()?;
        if !directory.is_dir() {
            return Err(Error::InvalidRequest(
                "The loop directory is not a directory.",
            ));
        }
        for session in self.registry.sessions()? {
            if session.body["kind"] == "loop"
                && session.body["loop_dir"].as_str() == directory.to_str()
            {
                return Ok((false, self.get_record(&session.id.to_string())?));
            }
        }
        self.check_spawn_limits(parent_session_id)?;
        let id = ws_core::ids::Ulid::new();
        let alias = self.next_alias()?;
        self.append(
            id,
            "session.created",
            json!({"alias":alias,"name":null,"workspace_id":null,
                "parent_session_id":parent_session_id,"kind":"loop","loop_dir":directory}),
        )?;
        Ok((true, self.get_record(&id.to_string())?))
    }
}

async fn start(
    State(store): State<Store>,
    Json(request): Json<LoopRequest>,
) -> std::result::Result<(axum::http::StatusCode, Json<SessionResponse>), ApiError> {
    let caller = request.parent_session_id;
    let (directory, run_id, script, _lock) = api::call(store.clone(), move |sessions| {
        sessions.record_call(caller, "loop.start", request.loop_dir.to_str())?;
        let (directory, state) = read_loop(&request.loop_dir)?;
        if state
            .get("awaitingMerge")
            .is_some_and(|value| !value.is_null() && value != false)
        {
            return Err(Error::InvalidRequest(
                "The loop awaits a milestone merge. Use loop-execute resume after the merge.",
            ));
        }
        let sub = directory.join("sub");
        fs::create_dir_all(&sub)?;
        let lock = ws_core::lockfile::open_lock_file(&sub.join("start.lock"))?;
        lock.try_lock()
            .map_err(|_| Error::InvalidRequest("Another caller is starting this loop."))?;
        for session in sessions.registry.sessions()? {
            if (session.body["loop_dir"].as_str() == directory.to_str()
                || state["wsSession"].as_str() == Some(session.id.to_string().as_str()))
                && session.body["kind"] == "headless"
                && !sessions.process_exited(session.id)?
            {
                return Err(Error::InvalidRequest(
                    "The loop already has a running orchestrator.",
                ));
            }
        }
        let script = script_path("LOOP_ORCHESTRATOR_SH", "loop-orchestrator.sh")?.canonicalize()?;
        Ok((
            directory,
            state["runId"].as_str().unwrap().to_owned(),
            script,
            lock,
        ))
    })
    .await?;
    let sub = directory.join("sub");
    let session = crate::supervisor::launch(
        store.clone(),
        crate::supervisor::CreateHeadless {
            headless: true,
            command: vec![
                "zsh".into(),
                script.to_string_lossy().into_owned(),
                "--run-id".into(),
                run_id.clone(),
                "--dir".into(),
                directory.to_string_lossy().into_owned(),
                "--prompt-file".into(),
                sub.join("prompt-seed.md").to_string_lossy().into_owned(),
                "--ws-session".into(),
            ],
            cwd: directory
                .parent()
                .ok_or(ApiError(Error::InvalidRequest(
                    "The loop directory has no parent.",
                )))?
                .into(),
            parent_session_id: caller,
            env: BTreeMap::new(),
            stdin: None,
            stdout: Some(sub.join("orchestrator.stdout.log")),
            stderr: Some(sub.join("orchestrator.stderr.log")),
            key: None,
        },
        json!({"loop_dir":directory,"run_id":run_id}),
    )
    .await?;
    let id = session.id;
    let registration = api::call(store.clone(), move |sessions| {
        let (_, mut state) = read_loop(&directory)?;
        state["wsSession"] = json!(id);
        atomic_write(
            &directory.join("state.json"),
            &serde_json::to_vec_pretty(&state).map_err(Error::Json)?,
        )?;
        let running = sessions
            .registry
            .latest_event(id, "session.running")?
            .ok_or(Error::InvalidRequest(
                "The orchestrator has no process record.",
            ))?;
        atomic_write(
            &sub.join("orch.pid"),
            format!("{}\n", running.body["process"]["pid"]).as_bytes(),
        )?;
        Ok(())
    })
    .await;
    if let Err(error) = registration {
        let _ = crate::supervisor::stop(store, id.to_string(), caller).await;
        return Err(error);
    }
    Ok((axum::http::StatusCode::CREATED, Json(session.into())))
}

async fn stop(
    State(store): State<Store>,
    Json(request): Json<LoopRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(
                request.parent_session_id,
                "loop.stop",
                request.loop_dir.to_str(),
            )?;
            let (directory, state) = read_loop(&request.loop_dir)?;
            let control = directory.join("control");
            fs::create_dir_all(&control)?;
            atomic_write(
                &control.join("pause"),
                format!(
                    "{}\n",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                )
                .as_bytes(),
            )?;
            Ok(json!({"loop_dir":directory,"run_id":state["runId"],"paused":true}))
        })
        .await?,
    ))
}

fn read_loop(directory: &Path) -> Result<(PathBuf, Value)> {
    if !directory.is_absolute() {
        return Err(Error::InvalidRequest(
            "The loop directory must be absolute.",
        ));
    }
    let directory = directory.canonicalize()?;
    let state: Value =
        serde_json::from_slice(&fs::read(directory.join("state.json"))?).map_err(Error::Json)?;
    if state["runId"]
        .as_str()
        .is_none_or(|id| id.trim().is_empty())
    {
        return Err(Error::InvalidRequest("The loop state has no runId."));
    }
    Ok((directory, state))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("tmp-{}", ws_core::ids::Ulid::new()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Error::Io)
}
