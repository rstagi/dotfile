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
            let Some(created) = self.registry.latest_event(entry.session_id, "session.created")? else {
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
        let review = child["role"].is_string();
        let kind = if review { "loop.review" } else { "loop.attempt" };
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
            body["exit"] = json!(entry.body["code"].as_i64().unwrap_or(-1));
        }
        self.append_daemon_loop_event(
            &owner.id.to_string(),
            kind,
            body.as_object().unwrap().clone(),
        )?;
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
        "attempts":[], "merges":[], "gates":[], "reviews":[], "notes":[],
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
                state[field].as_array_mut().unwrap().push(body);
            }
        }
    }
    state
}

fn promote_phase(phases: &mut Value, incoming: &Value) {
    let Some(n) = incoming["n"].as_u64() else {
        return;
    };
    let key = n.to_string();
    let ranks = ["todo", "claimed", "running", "done", "merged"];
    let rank = |phase: &Value| {
        ranks
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
            if name != "status" || ranks.iter().any(|status| value == status) {
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
        for _ in 0..2 {
            let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
            let (_, state) = request(&app, "GET", &uri, Value::Null).await;
            let attempts = state["attempts"].as_array().unwrap();
            assert_eq!(attempts.len(), 2, "{state}");
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
                json!({"phases":[{"n":1,"title":"Build", "status":"todo"}]}),
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
                json!({"phases":[{"n":1,"title":"Build", "status":"todo"}]}),
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
