//! Loop controls use the existing shell orchestrator and dashboard control files.
//! A loop session holds the Loop state as events. It has no process.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use axum::{
    Json, Router,
    extract::{Path as RoutePath, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ws_core::{ids::SessionId, log::LogEntry, registry::Session};

use crate::{
    api::{self, ApiError, SessionResponse, Store},
    sessions::{Error, Result, Sessions},
};

/// The phase statuses from lowest to highest. The Loop fold never lowers a status.
const PHASE_RANKS: [&str; 5] = ["todo", "claimed", "running", "done", "merged"];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopRequest {
    pub loop_dir: PathBuf,
    pub parent_session_id: Option<SessionId>,
}

/// Names the loop that owns a new session, and the phase attempt or review step it runs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopChild {
    /// ULID, alias or name of the loop session.
    #[serde(rename = "loop")]
    pub loop_id: String,
    pub phase: u64,
    pub attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<u64>,
}

impl LoopChild {
    /// Checks the derived start body before the daemon reserves the child session.
    pub(crate) fn validate_execution(&self, session: SessionId, key: &str) -> Result<()> {
        let child = serde_json::to_value(self).map_err(Error::Json)?;
        let (kind, body) = execution_event(&child, session, key, "started", None);
        crate::events::check_body(kind, &body).map_err(Error::InvalidBody)
    }
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/loops", get(list))
        .route("/v1/loops/{id}", get(fetch))
        .route("/v1/loops/create", post(create))
        .route("/v1/loops/start", post(start))
        .route("/v1/loops/stop", post(stop))
}

async fn list(
    State(store): State<Store>,
    Query(context): Query<api::CallContext>,
) -> std::result::Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(context.parent_session_id, "loop.list", None)?;
            sessions
                .registry
                .sessions()?
                .into_iter()
                .filter(|session| session.body["kind"] == "loop")
                .map(|session| sessions.loop_state(&session))
                .collect()
        })
        .await?,
    ))
}

async fn fetch(
    State(store): State<Store>,
    RoutePath(id): RoutePath<String>,
    Query(context): Query<api::CallContext>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(context.parent_session_id, "loop.get", Some(&id))?;
            let session = sessions.get_record(&id)?;
            if session.body["kind"] != "loop" {
                return Err(Error::InvalidRequest("The session is not a loop."));
            }
            sessions.loop_state(&session)
        })
        .await?,
    ))
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
    /// Records execution facts while the caller still holds the session store lock.
    pub(crate) fn record_loop_execution(&mut self, entry: &LogEntry) -> Result<()> {
        let state = match entry.event_type.as_str() {
            "session.created" => "started",
            "exit" | "session.finished" | "session.failed" | crate::supervisor::PROCESS_EXITED => {
                "exited"
            }
            _ => return Ok(()),
        };
        let created = if state == "started" {
            entry.clone()
        } else {
            let Some(created) = self
                .registry
                .latest_event(entry.session_id, "session.created")?
            else {
                return Ok(());
            };
            created
        };
        let Some(child) = created.body.get("loop").filter(|value| value.is_object()) else {
            return Ok(());
        };
        if created.body["kind"] != "headless" {
            return Ok(());
        }
        let Some(owner) = created.body["parent_session_id"].as_str() else {
            return Ok(());
        };
        let owner = self.get_record(owner)?;
        let session = entry.session_id.to_string();
        let key = created.body["key"].as_str().unwrap_or(&session);
        let (kind, body) = execution_event(
            child,
            entry.session_id,
            key,
            state,
            entry.body["code"].as_i64(),
        );
        let entries = ws_core::log::SessionLog::new(&self.state, owner.id, self.device_id)
            .read()?
            .entries;
        if entries.iter().any(|entry| {
            entry.event_type == kind
                && entry.body["source"] == "daemon"
                && entry.body["key"] == key
                && entry.body["state"] == state
        }) {
            return Ok(());
        }
        self.append_daemon_loop_event(&owner.id.to_string(), kind, body)?;
        Ok(())
    }

    /// Repairs a crash between the child log append and its derived Loop append.
    pub(crate) fn recover_loop_executions(&mut self) -> Result<()> {
        for session in self.registry.sessions()? {
            if session.body["kind"] != "headless" || !session.body["loop"].is_object() {
                continue;
            }
            let entries = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id)
                .read()?
                .entries;
            for entry in entries {
                self.record_loop_execution(&entry)?;
            }
        }
        Ok(())
    }

    /// Appends the `loop.phase` events that a Loop fact causes. A status never lowers.
    /// The caller holds the store lock, so the cause and its effect share one operation.
    pub(crate) fn derive_phase_status(&mut self, id: SessionId, entry: &LogEntry) -> Result<()> {
        if !["loop.attempt", "loop.review", "loop.merge"].contains(&entry.event_type.as_str()) {
            return Ok(());
        }
        let session = self.get_record(&id.to_string())?;
        let state = self.loop_state(&session)?;
        let Some((n, status)) = derived_status(&state, &entry.event_type, &entry.body) else {
            return Ok(());
        };
        let attempt = entry.body["attempt"].as_u64();
        let session_id = entry.body["session"].as_str();
        let body = json!({"n":n,"status":status,"attempt":attempt,"session":session_id});
        self.append_daemon_loop_event(
            &id.to_string(),
            "loop.phase",
            body.as_object().unwrap().clone(),
        )?;
        Ok(())
    }

    /// Repairs a crash between a Loop fact and the phase status that it causes.
    pub(crate) fn recover_phase_statuses(&mut self) -> Result<()> {
        for session in self.registry.sessions()? {
            if session.body["kind"] != "loop" {
                continue;
            }
            let entries = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id)
                .read()?
                .entries;
            for entry in entries {
                self.derive_phase_status(session.id, &entry)?;
            }
        }
        Ok(())
    }

    /// Refuses a child of a phase from a later milestone while a merge gate is open.
    pub(crate) fn check_milestone_gate(&self, loop_id: SessionId, phase: u64) -> Result<()> {
        let session = self.get_record(&loop_id.to_string())?;
        let state = self.loop_state(&session)?;
        let Some(open) = open_merge_gate(&state) else {
            return Ok(());
        };
        if state["phases"][phase.to_string()]["milestone"]
            .as_u64()
            .is_some_and(|milestone| milestone > open)
        {
            return Err(Error::MilestoneGateOpen);
        }
        Ok(())
    }

    fn loop_state(&self, session: &Session) -> Result<Value> {
        let log = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id).read()?;
        Ok(fold_loop(session, log.entries))
    }

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

/// Returns the phase and the status that a Loop fact implies, if that status is higher.
fn derived_status(state: &Value, kind: &str, body: &Value) -> Option<(u64, &'static str)> {
    let (n, status) = match kind {
        "loop.attempt" => {
            let n = body["n"].as_u64()?;
            match body["state"].as_str()? {
                "started" => (n, "running"),
                "result"
                    if body["outcome"] == "done"
                        && ["pass", "none"].iter().any(|v| body["verify"] == *v)
                        && state["phases"][n.to_string()]["kind"] == "work" =>
                {
                    (n, "done")
                }
                _ => return None,
            }
        }
        "loop.merge" if body["verify"] == "pass" => (body["n"].as_u64()?, "merged"),
        "loop.review" => {
            let n = body["phase"].as_u64()?;
            match body["state"].as_str()? {
                "started" => (n, "running"),
                "finished" if body["role"] == "final" && review_complete(state, n) => (n, "done"),
                _ => return None,
            }
        }
        _ => return None,
    };
    let rank = |status: &Value| PHASE_RANKS.iter().position(|s| status == *s).unwrap_or(0);
    let current = rank(&state["phases"][n.to_string()]["status"]);
    (PHASE_RANKS.iter().position(|s| *s == status)? > current).then_some((n, status))
}

/// A review phase is complete when each repository of its work phases has a final record.
fn review_complete(state: &Value, review: u64) -> bool {
    let phases = &state["phases"];
    if phases[review.to_string()]["kind"] != "review" {
        return false;
    }
    let repos: std::collections::BTreeSet<&str> = phases[review.to_string()]["depends_on"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|dependency| &phases[dependency.to_string()])
        .filter(|dependency| dependency["kind"] == "work")
        .map(|dependency| dependency["repo"].as_str().unwrap_or("none"))
        .collect();
    !repos.is_empty()
        && repos.iter().all(|repo| {
            state["reviews"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|record| {
                    record["phase"] == review
                        && record["role"] == "final"
                        && record["state"] == "finished"
                        && record["repo"] == *repo
                })
        })
}

fn execution_event(
    child: &Value,
    session: SessionId,
    key: &str,
    state: &str,
    exit: Option<i64>,
) -> (&'static str, serde_json::Map<String, Value>) {
    let review = child["role"].is_string();
    let kind = if review {
        "loop.review"
    } else {
        "loop.attempt"
    };
    let mut body = json!({
        "attempt":child["attempt"], "key":key, "session":session, "state":state,
    });
    body[if review { "phase" } else { "n" }] = child["phase"].clone();
    for field in ["repo", "role", "provider", "round", "inner"] {
        if let Some(value) = child.get(field) {
            body[field] = value.clone();
        }
    }
    if !review && state == "exited" {
        // A restart or a session completion can precede a known process status.
        body["exit"] = json!(exit.unwrap_or(-1));
    }
    (kind, body.as_object().unwrap().clone())
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
            loop_child: None,
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

/// Builds the Loop projection from the session log. Phase statuses only increase.
fn fold_loop(session: &Session, entries: Vec<LogEntry>) -> Value {
    let mut state = json!({
        "id":session.id, "alias":session.alias, "loop_dir":session.body["loop_dir"],
        "status":"planned", "plan":null, "phases":{}, "milestone":null,
        "merge_gates":{}, "attempts":[], "merges":[], "gates":[], "reviews":[], "notes":[],
        "decisions":[], "controls":[], "guards":[], "violations":[], "finish":null
    });
    for entry in entries {
        let body = entry.body;
        if !entry.event_type.starts_with("loop.") {
            continue;
        }
        if state["status"] == "planned" {
            state["status"] = json!("active");
        }
        match entry.event_type.as_str() {
            "loop.plan" => state["plan"] = body,
            "loop.phases" => {
                if let Some(phases) = body["phases"].as_array() {
                    for phase in phases {
                        promote_phase(&mut state["phases"], phase);
                    }
                }
            }
            "loop.phase" => promote_phase(&mut state["phases"], &body),
            "loop.milestone" => state["milestone"] = body["milestone"].clone(),
            "loop.finish" => {
                state["status"] = json!("finished");
                state["finish"] = body;
            }
            kind => {
                let field = match kind {
                    "loop.attempt" => "attempts",
                    "loop.merge" => "merges",
                    "loop.gate" => "gates",
                    "loop.review" => "reviews",
                    "loop.note" => "notes",
                    "loop.decision" => "decisions",
                    "loop.control" => "controls",
                    "loop.guard" => "guards",
                    "loop.violation" => "violations",
                    _ => continue,
                };
                if kind == "loop.control" && state["status"] != "finished" {
                    match body["action"].as_str() {
                        Some("pause") => state["status"] = json!("paused"),
                        Some("resume") => state["status"] = json!("active"),
                        _ => {}
                    }
                }
                if kind == "loop.gate"
                    && body["kind"] == "merge"
                    && let Some(milestone) = body["milestone"].as_u64()
                {
                    state["merge_gates"][milestone.to_string()] = body["state"].clone();
                }
                state[field].as_array_mut().unwrap().push(body);
            }
        }
    }
    if state["status"] == "active" && open_merge_gate(&state).is_some() {
        state["status"] = json!("awaiting-merge");
    }
    state
}

/// Returns the lowest milestone whose merge gate is open.
fn open_merge_gate(state: &Value) -> Option<u64> {
    state["merge_gates"]
        .as_object()?
        .iter()
        .filter(|(_, gate)| *gate == "open")
        .filter_map(|(milestone, _)| milestone.parse().ok())
        .min()
}

fn promote_phase(phases: &mut Value, incoming: &Value) {
    let Some(n) = incoming["n"].as_u64() else {
        return;
    };
    let key = n.to_string();
    let rank = |phase: &Value| {
        PHASE_RANKS
            .iter()
            .position(|status| phase["status"] == *status)
            .unwrap_or(0)
    };
    let current = &mut phases[&key];
    if rank(incoming) < rank(current) {
        return;
    }
    if !current.is_object() {
        *current = json!({"n":n, "status":"todo"});
    }
    if let Some(fields) = incoming.as_object() {
        for (name, value) in fields {
            if name != "status" || PHASE_RANKS.iter().any(|status| value == status) {
                current[name] = value.clone();
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

    use crate::sessions::Sessions;

    #[tokio::test]
    async fn invalid_review_metadata_leaves_no_child_reservation() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, owner) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":sandbox.path()}),
        )
        .await;
        let (status, _) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({
                "headless":true,"command":["/bin/true"],"cwd":sandbox.path(),"key":"r1",
                "loop":{"loop":owner["id"],"phase":117,"attempt":1,"repo":"owner/repo",
                    "role":"invalid","provider":"gpt","round":1,"inner":1},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, sessions) = request(&app, "GET", "/v1/sessions", Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{sessions}");
        assert_eq!(sessions.as_array().unwrap().len(), 1, "{sessions}");
        drop(app);
        assert!(Sessions::open(sandbox.path(), 1).is_ok());
    }

    #[tokio::test]
    async fn recovery_repairs_interrupted_loop_appends_once_and_preserves_later_writes() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let (_, owner) = sessions.create_loop_session(sandbox.path(), None).unwrap();
        drop(sessions);
        // Model a crash after each child fact, before the corresponding Loop append.
        let child = ws_core::ids::Ulid::new();
        let log = ws_core::log::SessionLog::new(sandbox.path(), child, owner.device_id);
        log.append(
            0,
            "session.created",
            json!({
                "alias":2,"kind":"headless","key":"p117-a1","parent_session_id":owner.id,
                "loop":{"loop":owner.id,"phase":117,"attempt":1},
            }),
        )
        .unwrap();
        log.append(0, "exit", json!({"code":3})).unwrap();
        log.append(0, "session.failed", json!({})).unwrap();
        let uri = format!("/v1/loops/{}", owner.id);
        for count in 0..2 {
            let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
            let (_, state) = request(&app, "GET", &uri, Value::Null).await;
            let attempts = state["attempts"].as_array().unwrap();
            assert_eq!(attempts.len(), 2, "{state}");
            assert_eq!(state["notes"].as_array().unwrap().len(), count);
            assert_eq!(attempts[0]["state"], "started");
            assert_eq!(attempts[1]["state"], "exited");
            assert_eq!(attempts[1]["exit"], 3);
            for attempt in attempts {
                assert_eq!(attempt["source"], "daemon");
                assert_eq!(attempt["session"], child.to_string());
                assert_eq!(attempt["key"], "p117-a1");
                assert!(attempt.get("verdict").is_none());
            }
            let (status, _) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{}/events", owner.id),
                json!({"type":"loop.note","body":{"text":"Recovery permits another write."}}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }
    }

    #[tokio::test]
    async fn loop_reads_fold_events_without_lowering_phase_status_and_survive_restart() {
        let sandbox = tempfile::tempdir().unwrap();
        let directory = sandbox.path().join("loop");
        std::fs::create_dir(&directory).unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, ordinary) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, created) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":directory}),
        )
        .await;
        let id = created["id"].as_str().unwrap();
        let uri = format!("/v1/loops/{id}");
        let (status, initial) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(initial["status"], "planned");
        assert_eq!(initial["phases"], json!({}));
        let events = [
            ("loop.plan", json!({"text":"# Plan", "sha":"abc"})),
            (
                "loop.phases",
                json!({"phases":[{"n":1,"title":"Build","kind":"work","output":"pr","milestone":1,
                    "gate":null,"repo":null,"branch":null,"depends_on":[],"status":"todo"}]}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"claimed","attempt":1,"session":null}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"running","attempt":1,"session":null}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"done","attempt":1,"session":null}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"merged","attempt":1,"session":null}),
            ),
            (
                "loop.phase",
                json!({"n":1,"status":"running","attempt":0,"session":null}),
            ),
            (
                "loop.phases",
                json!({"phases":[{"n":1,"title":"Build","kind":"work","output":"pr","milestone":1,
                    "gate":null,"repo":null,"branch":null,"depends_on":[],"status":"todo"}]}),
            ),
            ("loop.milestone", json!({"milestone":5})),
            ("loop.note", json!({"text":"Ready"})),
            ("loop.control", json!({"action":"pause"})),
        ];
        for (kind, body) in events {
            let (status, _) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":kind,"body":body}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{kind}");
            let (_, current) = request(&app, "GET", &uri, Value::Null).await;
            if kind == "loop.phase" && body["attempt"] == 0 {
                assert_eq!(current["phases"]["1"]["status"], "merged");
                assert_eq!(current["phases"]["1"]["attempt"], 1);
            }
        }
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["id"], created["id"]);
        assert_eq!(state["loop_dir"], json!(directory.canonicalize().unwrap()));
        assert_eq!(state["plan"]["sha"], "abc");
        assert_eq!(state["phases"]["1"]["status"], "merged");
        assert_eq!(state["phases"]["1"]["title"], "Build");
        assert_eq!(state["milestone"], 5);
        assert_eq!(state["notes"][0]["text"], "Ready");
        assert_eq!(state["status"], "paused");
        let (_, listed) = request(&app, "GET", "/v1/loops", Value::Null).await;
        assert_eq!(listed, json!([state]));
        let (status, _) = request(
            &app,
            "GET",
            &format!("/v1/loops/{}", ordinary["id"].as_str().unwrap()),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        drop(app);
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, recovered) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(recovered, state);
    }

    #[tokio::test]
    async fn loop_reads_keep_work_records_and_a_terminal_finish() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, empty) = request(&app, "GET", "/v1/loops", Value::Null).await;
        assert_eq!(empty, json!([]));
        let (_, created) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":sandbox.path()}),
        )
        .await;
        let id = created["id"].as_str().unwrap();
        let cases = [
            (
                "loop.attempt",
                "attempts",
                json!({"n":1,"attempt":1,"session":"child","state":"result","outcome":"done","verify":"pass"}),
            ),
            (
                "loop.merge",
                "merges",
                json!({"n":1,"repo":"owner/repo","head":"abc","verify":"pass"}),
            ),
            (
                "loop.gate",
                "gates",
                json!({"kind":"merge","state":"open","milestone":5,"prs":[]}),
            ),
            (
                "loop.review",
                "reviews",
                json!({"phase":2,"repo":"owner/repo","key":"r1","role":"final","provider":"gpt","round":1,"state":"finished","verdict":"APPROVE","session":"reviewer"}),
            ),
            (
                "loop.decision",
                "decisions",
                json!({"topic":"depth","default":"standard","choice":"max","reason":"Review the boundary."}),
            ),
            (
                "loop.guard",
                "guards",
                json!({"session":"child","rule":"write","command":"git push"}),
            ),
            (
                "loop.violation",
                "violations",
                json!({"id":"v1","rule":"write","severity":"hard","phase":1,"evidence":"guard"}),
            ),
        ];
        for (kind, _, body) in &cases {
            let (status, _) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":kind,"body":body}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{kind}");
        }
        for (kind, body) in [
            ("loop.finish", json!({"summary":"Complete"})),
            ("loop.control", json!({"action":"resume"})),
        ] {
            let (status, _) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{id}/events"),
                json!({"type":kind,"body":body}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }
        let (_, state) = request(&app, "GET", &format!("/v1/loops/{id}"), Value::Null).await;
        for (_, field, mut body) in cases {
            body["source"] = json!("agent");
            body["caller"] = Value::Null;
            assert_eq!(state[field], json!([body]), "{field}");
        }
        assert_eq!(state["status"], "finished");
        assert_eq!(state["finish"]["summary"], "Complete");
        assert_eq!(state["controls"][0]["action"], "resume");
        let (status, _) = request(
            &app,
            "GET",
            &format!("/v1/loops/{}", ws_core::ids::Ulid::new()),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    fn phase(n: u64, kind: &str, repo: Value, milestone: u64, depends_on: Value) -> Value {
        json!({"n":n,"title":format!("P{n}"),"kind":kind,"output":"pr","milestone":milestone,
            "gate":null,"repo":repo,"branch":null,"depends_on":depends_on,"status":"todo"})
    }

    async fn post(app: &Router, id: &str, kind: &str, body: Value) -> StatusCode {
        request(
            app,
            "POST",
            &format!("/v1/sessions/{id}/events"),
            json!({"type":kind,"body":body}),
        )
        .await
        .0
    }

    async fn loop_session(app: &Router, dir: &std::path::Path) -> (String, String) {
        let (_, created) = request(app, "POST", "/v1/loops/create", json!({"loop_dir":dir})).await;
        let id = created["id"].as_str().unwrap().to_owned();
        (id.clone(), format!("/v1/loops/{id}"))
    }

    fn attempt(n: u64, state: &str, extra: Value) -> Value {
        let mut body = json!({"n":n,"attempt":1,"session":"child","state":state});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    }

    fn final_review(phase: u64, repo: &str) -> Value {
        json!({"phase":phase,"repo":repo,"key":format!("f-{repo}"),"role":"final","round":1,
            "state":"finished","session":"s","provider":"claude","verdict":"APPROVE"})
    }

    #[tokio::test]
    async fn loop_phases_and_milestone_bodies_are_checked() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, _) = loop_session(&app, sandbox.path()).await;
        let good = phase(1, "work", Value::Null, 1, json!([]));
        assert_eq!(
            post(&app, &id, "loop.phases", json!({"phases":[good.clone()]})).await,
            StatusCode::CREATED
        );
        for (field, value) in [
            ("n", json!("1")),
            ("title", json!("")),
            ("kind", json!("merge")),
            ("output", json!("video")),
            ("milestone", json!("1")),
            ("gate", json!("park")),
            ("repo", json!(7)),
            ("branch", json!(7)),
            ("depends_on", json!(["a"])),
            ("status", json!("finished")),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert_eq!(
                post(&app, &id, "loop.phases", json!({"phases":[bad]})).await,
                StatusCode::BAD_REQUEST,
                "{field}"
            );
        }
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("depends_on");
        assert_eq!(
            post(&app, &id, "loop.phases", json!({"phases":[missing]})).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            post(&app, &id, "loop.milestone", json!({"milestone":"2"})).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            post(&app, &id, "loop.milestone", json!({"milestone":2})).await,
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn loop_gate_bodies_are_checked_by_kind() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, _) = loop_session(&app, sandbox.path()).await;
        for body in [
            json!({"kind":"merge","state":"open","milestone":1,"prs":[]}),
            json!({"kind":"merge","state":"merged","milestone":1,"prs":[]}),
            json!({"kind":"accept","state":"open","milestone":1,"phase":4,"artifacts":[]}),
            json!({"kind":"accept","state":"accepted","milestone":1,"phase":4,"artifacts":[]}),
            json!({"kind":"accept","state":"rejected","milestone":1,"phase":4,"artifacts":[]}),
        ] {
            assert_eq!(
                post(&app, &id, "loop.gate", body.clone()).await,
                StatusCode::CREATED,
                "{body}"
            );
        }
        for body in [
            json!({"kind":"merge","state":"open","prs":[]}),
            json!({"kind":"merge","state":"accepted","milestone":1,"prs":[]}),
            json!({"kind":"accept","state":"merged","milestone":1,"phase":4,"artifacts":[]}),
            json!({"kind":"accept","state":"open","milestone":1,"artifacts":[]}),
        ] {
            assert_eq!(
                post(&app, &id, "loop.gate", body.clone()).await,
                StatusCode::BAD_REQUEST,
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn the_daemon_derives_phase_statuses_from_attempts_merges_and_reviews() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        let phases = json!({"phases":[
            phase(1, "work", json!("o/a"), 1, json!([])),
            phase(2, "work", json!("o/b"), 1, json!([])),
            phase(3, "work", Value::Null, 1, json!([])),
            phase(4, "review", Value::Null, 1, json!([1, 2, 3])),
        ]});
        post(&app, &id, "loop.phases", phases).await;
        let status = |state: &Value, n: u64| state["phases"][n.to_string()]["status"].clone();
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 1), "todo");

        // A started attempt gives running, and a failed Verify gives no done.
        post(&app, &id, "loop.attempt", attempt(1, "started", json!({}))).await;
        let result = |verify: &str| attempt(1, "result", json!({"outcome":"done","verify":verify}));
        post(&app, &id, "loop.attempt", result("fail")).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 1), "running");
        post(&app, &id, "loop.attempt", result("pass")).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 1), "done");
        // An outcome other than done gives no status.
        post(&app, &id, "loop.attempt", attempt(2, "started", json!({}))).await;
        post(
            &app,
            &id,
            "loop.attempt",
            attempt(2, "result", json!({"outcome":"question","verify":"none"})),
        )
        .await;
        // Verify none is enough for a phase without a repository.
        post(
            &app,
            &id,
            "loop.attempt",
            attempt(3, "result", json!({"outcome":"done","verify":"none"})),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 2), "running");
        assert_eq!(status(&state, 3), "done");
        // A stale started event and a later merge.
        post(&app, &id, "loop.attempt", attempt(1, "started", json!({}))).await;
        post(
            &app,
            &id,
            "loop.merge",
            json!({"n":1,"repo":"o/a","head":"h","verify":"fail"}),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 1), "done");
        post(
            &app,
            &id,
            "loop.merge",
            json!({"n":1,"repo":"o/a","head":"h","verify":"pass"}),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 1), "merged");

        // A reviewer step and an attempt result of a review phase give no done.
        let reviewer = json!({"phase":4,"repo":"o/a","key":"r","role":"review","round":1,
            "state":"started","session":"s","provider":"gpt"});
        post(&app, &id, "loop.review", reviewer).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 4), "running");
        post(
            &app,
            &id,
            "loop.attempt",
            attempt(4, "result", json!({"outcome":"done","verify":"pass"})),
        )
        .await;
        // Two repositories (and the repository none) need one final record each.
        post(&app, &id, "loop.review", final_review(4, "o/a")).await;
        post(&app, &id, "loop.review", final_review(4, "o/b")).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 4), "running");
        post(&app, &id, "loop.review", final_review(4, "none")).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(status(&state, 4), "done");

        // Every derived event has the daemon as source and raises the status.
        let (_, log) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}/log?type=loop.phase"),
            Value::Null,
        )
        .await;
        let derived = log["entries"].as_array().unwrap();
        assert!(!derived.is_empty());
        let mut last = std::collections::HashMap::new();
        let ranks = ["todo", "claimed", "running", "done", "merged"];
        for entry in derived.iter().filter(|e| e["type"] == "loop.phase") {
            assert_eq!(entry["body"]["source"], "daemon");
            let rank = ranks
                .iter()
                .position(|r| entry["body"]["status"] == *r)
                .unwrap();
            let previous = last.insert(entry["body"]["n"].as_u64().unwrap(), rank);
            assert!(previous.is_none_or(|p| rank > p), "{entry}");
        }
    }

    #[tokio::test]
    async fn an_open_merge_gate_gives_awaiting_merge_and_refuses_later_milestone_spawns() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        post(
            &app,
            &id,
            "loop.phases",
            json!({"phases":[
                phase(1, "work", Value::Null, 1, json!([])),
                phase(2, "work", Value::Null, 2, json!([])),
            ]}),
        )
        .await;
        let spawn = |phase: u64, key: &str| {
            json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),"key":key,
                "loop":{"loop":id,"phase":phase,"attempt":1}})
        };
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["status"], "active");
        post(
            &app,
            &id,
            "loop.gate",
            json!({"kind":"merge","state":"open","milestone":1,"prs":[]}),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["status"], "awaiting-merge");
        let (status, error) = request(&app, "POST", "/v1/sessions", spawn(2, "k2")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        let (_, sessions) = request(&app, "GET", "/v1/sessions", Value::Null).await;
        assert_eq!(sessions.as_array().unwrap().len(), 1, "{sessions}");
        // The same milestone and a phase without a record still start.
        let (status, body) = request(&app, "POST", "/v1/sessions", spawn(1, "k1")).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        // A continuation of the orchestrator has no loop object and still starts.
        let plain = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),"parent_session_id":id});
        let (status, body) = request(&app, "POST", "/v1/sessions", plain).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        post(
            &app,
            &id,
            "loop.gate",
            json!({"kind":"merge","state":"merged","milestone":1,"prs":[]}),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["status"], "active");
        let (status, body) = request(&app, "POST", "/v1/sessions", spawn(2, "k2")).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
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
        (
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
        )
    }
}
