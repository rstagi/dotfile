//! Loop controls record events and bridge the runner control files.
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

pub(crate) mod guard;

/// The phase statuses from lowest to highest. The Loop fold never lowers a status.
const PHASE_RANKS: [&str; 5] = ["todo", "claimed", "running", "done", "merged"];
/// The phase fields that the latest `loop.phases` snapshot defines.
const PHASE_DEFINITION_FIELDS: [&str; 8] = [
    "title",
    "kind",
    "output",
    "milestone",
    "gate",
    "repo",
    "branch",
    "depends_on",
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoopRequest {
    pub loop_dir: PathBuf,
    pub plan_id: Option<String>,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    phase: Option<u64>,
    text: Option<String>,
    model: Option<String>,
    tier: Option<String>,
    rounds: Option<u8>,
    parent_session_id: Option<SessionId>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AckRequest {
    id: String,
    reason: String,
    parent_session_id: Option<SessionId>,
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
        .route("/v1/loops/{id}/check", get(check))
        .route("/v1/loops/create", post(create))
        .route("/v1/loops/guard", post(guard::run))
        .route("/v1/loops/ack", post(ack))
        .route("/v1/loops/start", post(start))
        .route("/v1/loops/stop", post(stop))
        .route("/v1/loops/{id}/{action}", post(control))
}

async fn ack(
    State(store): State<Store>,
    Json(request): Json<AckRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.ack_loop_violation(request)).await?,
    ))
}

async fn control(
    State(store): State<Store>,
    RoutePath((id, action)): RoutePath<(String, String)>,
    Json(request): Json<ControlRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    let entry = api::call(store, move |sessions| {
        sessions.control_loop(&id, &action, request)
    })
    .await?;
    Ok(Json(entry))
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
            let session = sessions.resolve_loop(&id)?;
            if session.body["kind"] != "loop" {
                return Err(Error::InvalidRequest("The session is not a loop."));
            }
            sessions.loop_state(&session)
        })
        .await?,
    ))
}

async fn check(
    State(store): State<Store>,
    RoutePath(id): RoutePath<String>,
    Query(context): Query<api::CallContext>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(context.parent_session_id, "loop.check", Some(&id))?;
            let session = sessions.resolve_loop(&id)?;
            if session.body["kind"] != "loop" {
                return Err(Error::InvalidRequest("The session is not a loop."));
            }
            let log =
                ws_core::log::SessionLog::new(&sessions.state, session.id, sessions.device_id)
                    .read()?;
            let mut result = crate::loop_check::check_entries(&log.entries);
            result["id"] = json!(session.id);
            Ok(result)
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
        sessions.create_loop_session(
            &request.loop_dir,
            request.plan_id.as_deref(),
            request.parent_session_id,
        )
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
    fn ack_loop_violation(&mut self, request: AckRequest) -> Result<Value> {
        self.record_call(request.parent_session_id, "loop.ack", Some(&request.id))?;
        if request.reason.trim().is_empty() {
            return Err(Error::InvalidRequest("Specify the acknowledgment reason."));
        }
        for session in self.registry.sessions()? {
            if session.body["kind"] != "loop" {
                continue;
            }
            let state = self.loop_state(&session)?;
            let Some(violation) = state["violations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == request.id && v["source"] == "daemon")
            else {
                continue;
            };
            if violation["severity"] == "hard" && request.parent_session_id.is_some() {
                return Err(Error::LoopControlConflict(
                    "Only the human can acknowledge a hard violation.",
                ));
            }
            let entry = self.append_daemon_loop_event(
                &session.id.to_string(),
                "loop.control",
                json!({"action":"ack","violation":request.id,"reason":request.reason,
                    "acknowledged_by":request.parent_session_id})
                .as_object()
                .unwrap()
                .clone(),
            )?;
            return serde_json::to_value(entry).map_err(Error::Json);
        }
        Err(Error::InvalidRequest("The loop violation does not exist."))
    }

    /// Records each new rule result once. The caller holds the session store lock.
    pub(crate) fn enforce_loop_rules(&mut self, id: SessionId) -> Result<()> {
        let session = self.get_record(&id.to_string())?;
        if session.body["kind"] != "loop" {
            return Ok(());
        }
        let entries = ws_core::log::SessionLog::new(&self.state, id, self.device_id)
            .read()?
            .entries;
        let report = crate::loop_check::check_entries(&entries);
        for result in report["violations"].as_array().unwrap() {
            if entries.iter().any(|entry| {
                entry.event_type == "loop.violation"
                    && entry.body["source"] == "daemon"
                    && entry.body["rule"] == result["rule"]
                    && entry.body["severity"] == result["severity"]
                    && entry.body["phase"] == result["phase"]
                    && entry.body["evidence"] == result["seqs"]
            }) {
                continue;
            }
            let body = json!({"id":ws_core::ids::Ulid::new(),"rule":result["rule"],
                "severity":result["severity"],"phase":result["phase"],
                "evidence":result["seqs"],"message":result["message"]});
            self.append_daemon_loop_event(
                &id.to_string(),
                "loop.violation",
                body.as_object().unwrap().clone(),
            )?;
            if result["severity"] == "hard" {
                crate::notify::loop_violation(id, &body);
            }
        }
        Ok(())
    }

    fn control_loop(&mut self, id: &str, action: &str, request: ControlRequest) -> Result<Value> {
        self.record_call(
            request.parent_session_id,
            &format!("loop.{action}"),
            Some(id),
        )?;
        let session = self.resolve_loop(id)?;
        if session.body["kind"] != "loop" {
            return Err(Error::InvalidRequest("The session is not a loop."));
        }
        if request.phase == Some(0) {
            return Err(Error::InvalidRequest("The phase must be positive."));
        }
        let directory = PathBuf::from(session.body["loop_dir"].as_str().unwrap());
        let (kind, body, file) = match action {
            "pause" | "resume" => {
                let name = request
                    .phase
                    .map_or_else(|| "pause".into(), |n| format!("pause-{n}"));
                (
                    "loop.control",
                    json!({"action":action,"phase":request.phase}),
                    Some((
                        directory.join("control").join(name),
                        action == "pause",
                        "paused\n".to_owned(),
                    )),
                )
            }
            "model" => {
                let phase = request
                    .phase
                    .ok_or(Error::InvalidRequest("Specify a phase."))?;
                let model = request
                    .model
                    .as_deref()
                    .filter(|model| valid_model(model))
                    .ok_or(Error::InvalidRequest(
                        "Specify a runner model as provider:model[+effort].",
                    ))?;
                (
                    "loop.control",
                    json!({"action":"model","phase":phase,"model":model}),
                    Some((
                        directory.join(format!("control/model-{phase}")),
                        true,
                        format!("{model}\n"),
                    )),
                )
            }
            "tier" => {
                let phase = request
                    .phase
                    .ok_or(Error::InvalidRequest("Specify a phase."))?;
                let state = self.loop_state(&session)?;
                let live = match fs::read(directory.join("state.json")) {
                    Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(Error::Json)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
                    Err(error) => return Err(Error::Io(error)),
                };
                let live_phase = &live["phases"][phase.to_string()];
                if live_phase["status"]
                    .as_str()
                    .is_some_and(|status| status != "todo")
                    || live_phase["attempt"]
                        .as_u64()
                        .is_some_and(|attempt| attempt > 0)
                    || state["phases"][phase.to_string()]["status"]
                        .as_str()
                        .is_some_and(|status| status != "todo")
                    || state["attempts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|record| record["n"] == phase)
                    || state["reviews"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|record| record["phase"] == phase)
                {
                    return Err(Error::LoopControlConflict(
                        "The review phase already started.",
                    ));
                }
                let tier = request
                    .tier
                    .as_deref()
                    .filter(|tier| ["standard", "max"].contains(tier))
                    .ok_or(Error::InvalidRequest(
                        "Use standard or max as the review tier.",
                    ))?;
                if request
                    .rounds
                    .is_some_and(|rounds| !(1..=5).contains(&rounds))
                {
                    return Err(Error::InvalidRequest(
                        "The review rounds must be between 1 and 5.",
                    ));
                }
                let script = script_path("LOOP_PLAN_TIER_MJS", "loop-plan-tier.mjs")?;
                let mut command = std::process::Command::new("node");
                command
                    .arg(script)
                    .arg("--plan")
                    .arg(directory.join("plan.md"))
                    .arg("--phase")
                    .arg(phase.to_string())
                    .arg("--tier")
                    .arg(tier);
                if let Some(rounds) = request.rounds {
                    command.arg("--rounds").arg(rounds.to_string());
                }
                let output = command.output()?;
                if !output.status.success() {
                    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                    if message.contains("already started") {
                        return Err(Error::LoopControlConflict(
                            "The review phase already started.",
                        ));
                    }
                    return Err(Error::InvalidBody(message));
                }
                let body = serde_json::from_slice(&output.stdout).map_err(Error::Json)?;
                ("loop.plan", body, None)
            }
            "accept" | "reject" => {
                let state = self.loop_state(&session)?;
                let gate = state["gates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|gate| gate["kind"] == "accept")
                    .filter(|gate| gate["state"] == "open")
                    .ok_or(Error::LoopControlConflict(
                        "The loop has no open accept gate.",
                    ))?;
                (
                    "loop.gate",
                    json!({
                        "kind":"accept",
                        "state":if action == "accept" {"accepted"} else {"rejected"},
                        "milestone":gate["milestone"],
                        "phase":gate["phase"],
                        "artifacts":gate["artifacts"],
                    }),
                    None,
                )
            }
            "note" => {
                let phase = request
                    .phase
                    .ok_or(Error::InvalidRequest("Specify a phase."))?;
                let text = request
                    .text
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .ok_or(Error::InvalidRequest("Specify the note text."))?;
                ("loop.note", json!({"phase":phase,"text":text}), None)
            }
            _ => return Err(Error::InvalidRequest("The loop control is unknown.")),
        };
        crate::events::check_body(kind, body.as_object().unwrap()).map_err(Error::InvalidBody)?;
        if let Some((path, write, text)) = file {
            if write {
                fs::create_dir_all(path.parent().unwrap())?;
                atomic_write(&path, text.as_bytes())?;
            } else if let Err(error) = fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(Error::Io(error));
            }
        }
        let entry = self.append_daemon_loop_event(
            &session.id.to_string(),
            kind,
            body.as_object().unwrap().clone(),
        )?;
        serde_json::to_value(entry).map_err(Error::Json)
    }

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
            self.enforce_loop_rules(session.id)?;
        }
        Ok(())
    }

    /// Refuses a child of a phase that waits for an accept gate, or of a later milestone while a
    /// merge gate is open.
    pub(crate) fn check_milestone_gate(&self, loop_id: SessionId, phase: u64) -> Result<()> {
        let session = self.get_record(&loop_id.to_string())?;
        let state = self.loop_state(&session)?;
        if blocked_by_accept_gate(&state, phase) {
            return Err(Error::AcceptGateOpen);
        }
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

    /// Refuses phase execution for a pause or a hard violation without an acknowledgment.
    pub(crate) fn check_loop_pause(&mut self, id: SessionId, phase: u64) -> Result<()> {
        // Recover a failed durable append before reading the spawn guards.
        self.record_call(None, "loop.spawn", None)?;
        let state = self.loop_state(&self.get_record(&id.to_string())?)?;
        if state["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|violation| {
                violation["source"] == "daemon"
                    && violation["severity"] == "hard"
                    && !state["controls"].as_array().unwrap().iter().any(|control| {
                        control["source"] == "daemon"
                            && control["action"] == "ack"
                            && control["violation"] == violation["id"]
                            && control["acknowledged_by"].is_null()
                    })
            })
        {
            return Err(Error::LoopControlConflict(
                "The loop has a hard violation without a human acknowledgment.",
            ));
        }
        if state["pauses"]["all"] == true || state["pauses"][phase.to_string()] == true {
            return Err(Error::LoopControlConflict(
                "The loop or the phase is paused.",
            ));
        }
        Ok(())
    }

    fn loop_state(&self, session: &Session) -> Result<Value> {
        let log = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id).read()?;
        let plan_id = loop_plan_id(session, &log.entries);
        let mut state = fold_loop(session, log.entries);
        state["plan_id"] = json!(plan_id);
        Ok(state)
    }

    /// Resolves the session identity or the plan identity of a loop.
    pub(crate) fn resolve_loop(&self, input: &str) -> Result<Session> {
        match self.get_record(input) {
            Ok(session) => return Ok(session),
            Err(Error::Registry(ws_core::registry::Error::NotFound(_))) => {}
            Err(error) => return Err(error),
        }
        for session in self.registry.sessions()? {
            if session.body["kind"] == "loop" {
                let entries =
                    ws_core::log::SessionLog::new(&self.state, session.id, self.device_id)
                        .read()?
                        .entries;
                if loop_plan_id(&session, &entries).as_deref() == Some(input) {
                    return Ok(session);
                }
            }
        }
        Err(Error::Registry(ws_core::registry::Error::NotFound(
            input.into(),
        )))
    }

    /// Returns `(true, session)` for a new loop session and `(false, session)` for the session
    /// that already holds this plan id or canonical loop directory.
    pub fn create_loop_session(
        &mut self,
        loop_dir: &Path,
        plan_id: Option<&str>,
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
        let plan_id = match plan_id {
            Some(id) => Some(id.trim().to_owned()).filter(|id| !id.is_empty()),
            None => fs::read_to_string(directory.join("plan.md"))
                .ok()
                .and_then(|text| plan_header_value(&text, "planId")),
        }
        .ok_or(Error::InvalidRequest(
            "Specify a plan id in the plan header or with --plan-id.",
        ))?;
        let mut directory_match = None;
        for session in self.registry.sessions()? {
            if session.body["kind"] != "loop" {
                continue;
            }
            let entries = ws_core::log::SessionLog::new(&self.state, session.id, self.device_id)
                .read()?
                .entries;
            let same_directory = session.body["loop_dir"].as_str().is_some_and(|path| {
                Path::new(path)
                    .canonicalize()
                    .is_ok_and(|path| path == directory)
            });
            if loop_plan_id(&session, &entries).as_deref() == Some(&plan_id) {
                return Ok((false, session));
            }
            if same_directory {
                directory_match = Some(session);
            }
        }
        if let Some(session) = directory_match {
            return Ok((false, session));
        }
        self.check_spawn_limits(parent_session_id)?;
        let id = ws_core::ids::Ulid::new();
        let alias = self.next_alias()?;
        self.append(
            id,
            "session.created",
            json!({"alias":alias,"name":null,"workspace_id":null,
                "parent_session_id":parent_session_id,"kind":"loop","loop_dir":directory,"plan_id":plan_id}),
        )?;
        Ok((true, self.get_record(&id.to_string())?))
    }
}

/// Older sessions obtain their plan identity from the latest plan event, then the file.
fn loop_plan_id(session: &Session, entries: &[LogEntry]) -> Option<String> {
    session.body["plan_id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            entries
                .iter()
                .rev()
                .find(|entry| entry.event_type == "loop.plan")
                .and_then(|entry| entry.body["text"].as_str())
                .and_then(|text| plan_header_value(text, "planId"))
        })
        .or_else(|| {
            session.body["loop_dir"]
                .as_str()
                .and_then(|path| fs::read_to_string(Path::new(path).join("plan.md")).ok())
                .and_then(|text| plan_header_value(&text, "planId"))
        })
}

/// Reads a nonempty field from the canonical plan header.
fn plan_header_value(text: &str, field: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "<!-- loop-plan" {
        return None;
    }
    lines
        .take_while(|line| line.trim() != "-->")
        .filter_map(|line| line.split_once(':'))
        .find(|(key, value)| key.trim() == field && !value.trim().is_empty())
        .map(|(_, value)| value.trim().to_owned())
}

fn valid_model(model: &str) -> bool {
    let Some((provider, leg)) = model.split_once(':') else {
        return false;
    };
    if !["codex", "claude", "grok"].contains(&provider) {
        return false;
    }
    let leg = leg.strip_prefix('@').unwrap_or(leg);
    let mut parts = leg.split('+');
    let identifier = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    identifier(parts.next().unwrap_or_default())
        && parts
            .next()
            .is_none_or(|effort| effort.split(',').all(identifier))
        && parts.next().is_none()
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
    if skills_engine(&request.loop_dir).map_err(ApiError)? {
        return start_skills(store, request).await;
    }
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

/// Reads the engine only from the canonical plan header.
fn skills_engine(directory: &Path) -> Result<bool> {
    if !directory.is_absolute() {
        return Ok(false);
    }
    let plan = match fs::read_to_string(directory.join("plan.md")) {
        Ok(plan) => plan,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut lines = plan.lines();
    if lines.next().map(str::trim) != Some("<!-- loop-plan") {
        return Ok(false);
    }
    Ok(lines
        .take_while(|line| line.trim() != "-->")
        .filter_map(|line| line.split_once(':'))
        .any(|(key, value)| key.trim() == "engine" && value.trim() == "skills"))
}

async fn start_skills(
    store: Store,
    request: LoopRequest,
) -> std::result::Result<(axum::http::StatusCode, Json<SessionResponse>), ApiError> {
    let (directory, owner, _lock) = api::call(store.clone(), move |sessions| {
        sessions.record_call(
            request.parent_session_id,
            "loop.start",
            request.loop_dir.to_str(),
        )?;
        let directory = request.loop_dir.canonicalize()?;
        let sub = directory.join("sub");
        fs::create_dir_all(&sub)?;
        let lock = ws_core::lockfile::open_lock_file(&sub.join("start.lock"))?;
        lock.try_lock()
            .map_err(|_| Error::InvalidRequest("Another caller is starting this loop."))?;
        let state = match fs::read(directory.join("state.json")) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(Error::Json)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(error) => return Err(error.into()),
        };
        if state
            .get("awaitingMerge")
            .is_some_and(|value| !value.is_null() && value != false)
        {
            return Err(Error::InvalidBody(format!(
                "The loop awaits a milestone merge. Run loop-milestone.sh resume --dir {}.",
                shell_words::quote(&directory.to_string_lossy())
            )));
        }
        let (_, owner) = sessions.create_loop_session(
            &directory,
            request.plan_id.as_deref(),
            request.parent_session_id,
        )?;
        for session in sessions.registry.sessions()? {
            let orchestrator_child = session.body["parent_session_id"] == json!(owner.id)
                && session.body["loop"].is_null();
            if session.body["kind"] == "headless"
                && (session.body["loop_dir"].as_str() == directory.to_str()
                    || orchestrator_child
                    || state["wsSession"].as_str() == Some(session.id.to_string().as_str()))
                && !sessions.process_exited(session.id)?
            {
                return Err(Error::InvalidRequest(
                    "The loop already has a running orchestrator.",
                ));
            }
        }
        Ok((directory, owner.id, lock))
    })
    .await?;
    let prompt = format!(
        "Use the loop skill. The loop directory is {}. The loop session is {owner}. Continue until the loop reaches a gate or finishes.",
        directory.display()
    );
    // Preparation runs after the supervisor records the child and its loop parent.
    // Pass data as arguments so paths and prompts cannot become shell commands.
    let wrapper = r#"set -eu
prepared=$("$1" agent prepare --agent claude --mode headless --session "$WS_SESSION_ID" --format sh)
prompt=$2
eval "$prepared"
print_flag=$1
shift
exec "$WS_AGENT_PROGRAM" "$print_flag" "$prompt" "$@"
"#;
    let sub = directory.join("sub");
    let session = crate::supervisor::launch(
        store,
        crate::supervisor::CreateHeadless {
            headless: true,
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                wrapper.into(),
                "ws-loop".into(),
                std::env::current_exe()
                    .map_err(Error::Io)
                    .map_err(ApiError)?
                    .to_string_lossy()
                    .into_owned(),
                prompt,
            ],
            cwd: directory
                .parent()
                .ok_or(ApiError(Error::InvalidRequest(
                    "The loop directory has no parent.",
                )))?
                .into(),
            parent_session_id: Some(owner),
            env: BTreeMap::new(),
            stdin: None,
            stdout: Some(sub.join("orchestrator.stdout.log")),
            stderr: Some(sub.join("orchestrator.stderr.log")),
            key: None,
            loop_child: None,
        },
        json!({"loop_dir":directory,"loop_role":"orchestrator"}),
    )
    .await?;
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
        "plan_id":session.body["plan_id"],
        "status":"planned", "plan":null, "phases":{}, "milestone":null,
        "merge_gates":{}, "attempts":[], "merges":[], "gates":[], "reviews":[], "notes":[],
        "decisions":[], "controls":[], "pauses":{}, "guards":[], "violations":[], "finish":null
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
                    replace_phases(&mut state["phases"], phases);
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
                if kind == "loop.control"
                    && let Some("pause" | "resume") = body["action"].as_str()
                {
                    let key = body["phase"]
                        .as_u64()
                        .map_or_else(|| "all".into(), |n| n.to_string());
                    state["pauses"][key] = json!(body["action"] == "pause");
                    if state["status"] != "finished" {
                        state["status"] = if state["pauses"]
                            .as_object()
                            .unwrap()
                            .values()
                            .any(|paused| *paused == true)
                        {
                            json!("paused")
                        } else {
                            json!("active")
                        };
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

/// Tells whether the phase depends on a review phase whose latest accept gate is open or rejected.
fn blocked_by_accept_gate(state: &Value, phase: u64) -> bool {
    let Some(dependencies) = state["phases"][phase.to_string()]["depends_on"].as_array() else {
        return false;
    };
    dependencies.iter().filter_map(Value::as_u64).any(|n| {
        state["gates"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|gate| gate["kind"] == "accept" && gate["phase"] == n)
            .is_some_and(|gate| gate["state"] == "open" || gate["state"] == "rejected")
    })
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

/// Applies a `loop.phases` snapshot. The snapshot owns the phase list and the definitions.
/// A phase keeps its progress records and its highest status.
fn replace_phases(phases: &mut Value, snapshot: &[Value]) {
    let previous = std::mem::replace(phases, json!({}));
    for incoming in snapshot {
        let Some(n) = incoming["n"].as_u64() else {
            continue;
        };
        let key = n.to_string();
        let mut definition = incoming.clone();
        if let Some(old) = previous[&key].as_object() {
            let fields = definition.as_object_mut().unwrap();
            for (name, value) in old {
                if !PHASE_DEFINITION_FIELDS.contains(&name.as_str()) && name != "status" {
                    fields.entry(name.clone()).or_insert_with(|| value.clone());
                }
            }
            if phase_rank(&previous[&key]) > phase_rank(incoming) {
                fields.insert("status".into(), previous[&key]["status"].clone());
            }
        }
        phases[&key] = definition;
    }
}

fn phase_rank(phase: &Value) -> usize {
    PHASE_RANKS
        .iter()
        .position(|status| phase["status"] == *status)
        .unwrap_or(0)
}

fn promote_phase(phases: &mut Value, incoming: &Value) {
    let Some(n) = incoming["n"].as_u64() else {
        return;
    };
    let key = n.to_string();
    let current = &mut phases[&key];
    if phase_rank(incoming) < phase_rank(current) {
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
    async fn enforcement_blocks_hard_violations_until_the_human_acknowledges() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        assert_eq!(
            post(
                &app,
                &id,
                "loop.phase",
                json!({"n":1,"status":"merged","session":null})
            )
            .await,
            StatusCode::CREATED
        );
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        let violations = state["violations"].as_array().unwrap();
        let hard = violations.iter().find(|v| v["severity"] == "hard").unwrap();
        assert_eq!(hard["source"], "daemon");
        assert_eq!(hard["rule"], "merged-needs-verify");
        let violation = hard["id"].clone();
        post(&app, &id, "loop.note", json!({"text":"Check again."})).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(
            state["violations"].as_array().unwrap().len(),
            violations.len()
        );
        let spawn = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"blocked","loop":{"loop":id,"phase":2,"attempt":1}});
        let (status, _) = request(&app, "POST", "/v1/sessions", spawn.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let continuation = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "parent_session_id":id});
        let (_, agent) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let phase_spawn = json!({"parent_session_id":agent["id"],"phase":{"command":["/bin/true"]},
            "repo":sandbox.path(),"base":"main","caps_required":[],"max_runtime":30,
            "loop":{"loop":id,"phase":2,"attempt":1}});
        assert_eq!(
            request(&app, "POST", "/v1/phases/spawn", phase_spawn)
                .await
                .0,
            StatusCode::CONFLICT
        );
        let ack = json!({"id":violation,"reason":"I checked the merge."});
        let mut agent_ack = ack.clone();
        agent_ack["parent_session_id"] = agent["id"].clone();
        assert_eq!(
            request(&app, "POST", "/v1/loops/ack", agent_ack).await.0,
            StatusCode::CONFLICT
        );
        // An event append cannot bypass the human-only route.
        assert_eq!(
            post(
                &app,
                &id,
                "loop.control",
                json!({"action":"ack","id":violation,
            "reason":"Bypass"})
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        let (status, entry) = request(&app, "POST", "/v1/loops/ack", ack).await;
        assert_eq!(status, StatusCode::OK, "{entry}");
        assert_eq!(entry["type"], "loop.control");
        assert_eq!(entry["body"]["action"], "ack");
        assert_eq!(entry["body"]["violation"], violation);
        assert_eq!(entry["body"]["reason"], "I checked the merge.");
        // The acknowledgment and the duplicate suppression survive a restart.
        drop(app);
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        post(&app, &id, "loop.note", json!({"text":"After restart."})).await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(
            state["violations"].as_array().unwrap().len(),
            violations.len()
        );
        // A new hard result blocks work again, but the orchestrator can continue.
        post(
            &app,
            &id,
            "loop.phase",
            json!({"n":3,"status":"merged","session":null}),
        )
        .await;
        assert_eq!(
            request(&app, "POST", "/v1/sessions", continuation).await.0,
            StatusCode::CREATED
        );
        assert_eq!(
            request(&app, "POST", "/v1/sessions", spawn.clone()).await.0,
            StatusCode::CONFLICT
        );
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        let new_id = state["violations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["severity"] == "hard" && v["phase"] == 3)
            .unwrap()["id"]
            .clone();
        assert_eq!(
            request(
                &app,
                "POST",
                "/v1/loops/ack",
                json!({"id":new_id,"reason":"I checked the second merge."})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            request(&app, "POST", "/v1/sessions", spawn).await.0,
            StatusCode::CREATED
        );
        let next = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"after-ack","loop":{"loop":id,"phase":2,"attempt":2}});
        assert_eq!(
            request(&app, "POST", "/v1/sessions", next).await.0,
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn enforcement_recovers_a_missing_violation_before_a_spawn() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let (_, owner) = sessions
            .create_loop_session(sandbox.path(), Some("fixture-plan"), None)
            .unwrap();
        // Model a durable fact whose derived violation append failed.
        ws_core::log::SessionLog::new(sandbox.path(), owner.id, owner.device_id)
            .append(
                0,
                "loop.phase",
                json!({"n":1,"status":"merged","session":null,"source":"tool"}),
            )
            .unwrap();
        sessions.needs_rebuild = true;
        let app = crate::api::router(sessions);
        let spawn = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"after-failure","loop":{"loop":owner.id,"phase":2,"attempt":1}});
        let (status, body) = request(&app, "POST", "/v1/sessions", spawn).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let (_, state) =
            request(&app, "GET", &format!("/v1/loops/{}", owner.id), Value::Null).await;
        assert_eq!(state["violations"].as_array().unwrap().len(), 1);
        assert_eq!(state["violations"][0]["rule"], "merged-needs-verify");
    }

    #[tokio::test]
    async fn enforcement_soft_violations_never_block_phase_or_keyed_spawns() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        post(
            &app,
            &id,
            "loop.guard",
            json!({"session":"child","rule":"write","command":"git push"}),
        )
        .await;
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["violations"].as_array().unwrap().len(), 1);
        assert_eq!(state["violations"][0]["severity"], "soft");
        let (_, agent) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let phase_spawn = json!({"parent_session_id":agent["id"],"phase":{"command":["/bin/true"]},
            "repo":sandbox.path(),"base":"main","caps_required":[],"max_runtime":30,
            "loop":{"loop":id,"phase":1,"attempt":1}});
        let (code, body) = request(&app, "POST", "/v1/phases/spawn", phase_spawn).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
        let spawn = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"soft","loop":{"loop":id,"phase":2,"attempt":1}});
        assert_eq!(
            request(&app, "POST", "/v1/sessions", spawn).await.0,
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn a_pause_after_finish_preserves_terminal_status_and_blocks_spawns() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        post(&app, &id, "loop.finish", json!({})).await;
        let (code, _) = request(&app, "POST", &format!("{uri}/pause"), json!({})).await;
        assert_eq!(code, StatusCode::OK);
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["status"], "finished");
        let (code, _) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({
            "headless":true,"command":["/bin/true"],"cwd":sandbox.path(),"key":"after-finish",
            "loop":{"loop":id,"phase":1,"attempt":1}}),
        )
        .await;
        assert_eq!(code, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn accept_controls_close_only_the_open_gate_and_copy_its_identity() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        let (code, _) = request(&app, "POST", &format!("{uri}/accept"), json!({})).await;
        assert_eq!(code, StatusCode::CONFLICT);
        for (action, state, milestone, phase) in
            [("accept", "accepted", 3, 9), ("reject", "rejected", 4, 12)]
        {
            post(
                &app,
                &id,
                "loop.gate",
                json!({"kind":"accept","state":"open",
                "milestone":milestone,"phase":phase,"artifacts":["report.md"]}),
            )
            .await;
            let (code, entry) = request(&app, "POST", &format!("{uri}/{action}"), json!({})).await;
            assert_eq!(code, StatusCode::OK, "{entry}");
            assert_eq!(entry["type"], "loop.gate");
            assert_eq!(entry["body"]["source"], "daemon");
            assert_eq!(entry["body"]["state"], state);
            assert_eq!(entry["body"]["milestone"], milestone);
            assert_eq!(entry["body"]["phase"], phase);
            assert_eq!(entry["body"]["artifacts"], json!(["report.md"]));
            let (code, _) = request(&app, "POST", &format!("{uri}/{action}"), json!({})).await;
            assert_eq!(code, StatusCode::CONFLICT);
        }
    }

    #[tokio::test]
    async fn skills_controls_bridge_files_and_block_new_children_until_resume() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        let control = format!("{uri}/pause");
        let (code, body) = request(&app, "POST", &control, json!({"phase":7})).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert!(sandbox.path().join("control/pause-7").exists());
        let (_, caller) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let phase_spawn = |phase| {
            json!({"parent_session_id":caller["id"],
            "phase":{"command":["/bin/true"]},"repo":sandbox.path(),"base":"HEAD",
            "caps_required":[],"max_runtime":30,"loop":{"loop":id,"phase":phase,"attempt":1}})
        };
        let (code, body) = request(&app, "POST", "/v1/phases/spawn", phase_spawn(7)).await;
        assert_eq!(code, StatusCode::CONFLICT, "{body}");
        let (code, body) = request(&app, "POST", "/v1/phases/spawn", phase_spawn(8)).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
        let spawn = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"p7-a1","loop":{"loop":id,"phase":7,"attempt":1}});
        let (code, _) = request(&app, "POST", "/v1/sessions", spawn.clone()).await;
        assert_eq!(code, StatusCode::CONFLICT);
        request(&app, "POST", &format!("{uri}/pause"), json!({})).await;
        request(&app, "POST", &format!("{uri}/resume"), json!({"phase":7})).await;
        assert!(!sandbox.path().join("control/pause-7").exists());
        let (code, _) = request(&app, "POST", "/v1/sessions", spawn.clone()).await;
        assert_eq!(code, StatusCode::CONFLICT);
        request(&app, "POST", &format!("{uri}/resume"), json!({})).await;
        assert!(!sandbox.path().join("control/pause").exists());
        let (code, body) = request(
            &app,
            "POST",
            &format!("{uri}/model"),
            json!({"phase":7,"model":"claude:claude-opus-5-5"}),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(
            std::fs::read_to_string(sandbox.path().join("control/model-7")).unwrap(),
            "claude:claude-opus-5-5\n"
        );
        let (code, body) = request(
            &app,
            "POST",
            &format!("{uri}/note"),
            json!({"phase":7,"text":"Use the new contract."}),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["notes"][0]["phase"], 7);
        assert_eq!(state["notes"][0]["text"], "Use the new contract.");
        assert_eq!(state["controls"][4]["model"], "claude:claude-opus-5-5");
        let (code, body) = request(&app, "POST", "/v1/sessions", spawn).await;
        assert_eq!(code, StatusCode::CREATED, "{body}");
        request(&app, "POST", &format!("{uri}/pause"), json!({"phase":7})).await;
        let duplicate = json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),
            "key":"p7-a1","loop":{"loop":id,"phase":7,"attempt":1}});
        let (code, _) = request(&app, "POST", "/v1/sessions", duplicate).await;
        assert_eq!(code, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn invalid_review_metadata_leaves_no_child_reservation() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, owner) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":sandbox.path(),"plan_id":format!("fixture:{}", sandbox.path().display())}),
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
        let (_, owner) = sessions
            .create_loop_session(sandbox.path(), Some("fixture-plan"), None)
            .unwrap();
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
            json!({"loop_dir":directory,"plan_id":format!("fixture:{}", directory.display())}),
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
    async fn a_new_phases_snapshot_replaces_definitions_and_keeps_the_highest_status() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        let old = |n: u64| phase(n, "work", Value::Null, 1, json!([]));
        let first = json!({"phases":[old(1), old(2)]});
        assert_eq!(
            post(&app, &id, "loop.phases", first).await,
            StatusCode::CREATED
        );
        let running = json!({"n":1,"status":"running","attempt":2,"session":"child"});
        assert_eq!(
            post(&app, &id, "loop.phase", running).await,
            StatusCode::CREATED
        );
        let mut replanned = old(2);
        replanned["title"] = json!("Renamed");
        replanned["depends_on"] = json!([3]);
        replanned["milestone"] = json!(2);
        let mut third = old(3);
        third["status"] = json!("done");
        let snapshot = json!({"phases":[replanned, third, old(1)]});
        assert_eq!(
            post(&app, &id, "loop.phases", snapshot).await,
            StatusCode::CREATED
        );
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["phases"]["1"]["status"], "running");
        assert_eq!(state["phases"]["1"]["attempt"], 2);
        assert_eq!(state["phases"]["2"]["title"], "Renamed");
        assert_eq!(state["phases"]["2"]["depends_on"], json!([3]));
        assert_eq!(state["phases"]["2"]["milestone"], 2);
        assert_eq!(state["phases"]["3"]["status"], "done");

        let mut retitled = old(1);
        retitled["title"] = json!("New title");
        retitled["depends_on"] = json!([2]);
        let snapshot = json!({"phases":[retitled]});
        assert_eq!(
            post(&app, &id, "loop.phases", snapshot).await,
            StatusCode::CREATED
        );
        let (_, state) = request(&app, "GET", &uri, Value::Null).await;
        assert_eq!(state["phases"]["1"]["title"], "New title");
        assert_eq!(state["phases"]["1"]["depends_on"], json!([2]));
        assert_eq!(state["phases"]["1"]["status"], "running");
        assert_eq!(state["phases"].as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn loop_phases_accept_each_plan_output_kind_and_refuse_document() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, _) = loop_session(&app, sandbox.path()).await;
        for output in ["pr", "file", "doc", "output.md"] {
            let mut work = phase(1, "work", Value::Null, 1, json!([]));
            work["output"] = json!(output);
            assert_eq!(
                post(&app, &id, "loop.phases", json!({"phases":[work]})).await,
                StatusCode::CREATED,
                "{output}"
            );
        }
        let mut work = phase(1, "work", Value::Null, 1, json!([]));
        work["output"] = json!("document");
        assert_eq!(
            post(&app, &id, "loop.phases", json!({"phases":[work]})).await,
            StatusCode::BAD_REQUEST
        );
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
            json!({"loop_dir":sandbox.path(),"plan_id":format!("fixture:{}", sandbox.path().display())}),
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
            if field == "violations" {
                assert!(state[field].as_array().unwrap().contains(&body));
            } else {
                assert_eq!(state[field], json!([body]), "{field}");
            }
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
        let (_, created) = request(
            app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir":dir,"plan_id":format!("fixture:{}", dir.display())}),
        )
        .await;
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

    #[tokio::test]
    async fn an_open_or_rejected_accept_gate_refuses_spawns_of_dependent_phases() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (id, uri) = loop_session(&app, sandbox.path()).await;
        post(
            &app,
            &id,
            "loop.phases",
            json!({"phases":[
                phase(1, "work", Value::Null, 1, json!([])),
                phase(2, "review", Value::Null, 1, json!([1])),
                phase(3, "work", Value::Null, 1, json!([2])),
                phase(4, "work", Value::Null, 1, json!([1])),
            ]}),
        )
        .await;
        let spawn = |phase: u64, key: &str| {
            json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.path(),"key":key,
                "loop":{"loop":id,"phase":phase,"attempt":1}})
        };
        let gate = |state: &str, artifacts: Value| json!({"kind":"accept","state":state,"milestone":1,"phase":2,"artifacts":artifacts});
        post(&app, &id, "loop.gate", gate("open", json!(["a.md"]))).await;
        let (status, error) = request(&app, "POST", "/v1/sessions", spawn(3, "k3")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["error"]["code"], "accept_gate_open", "{error}");
        // The gate phase and phases that do not depend on it still start.
        let (status, body) = request(&app, "POST", "/v1/sessions", spawn(4, "k4")).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, _) = request(&app, "POST", &format!("{uri}/reject"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let (status, error) = request(&app, "POST", "/v1/sessions", spawn(3, "k3")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        post(&app, &id, "loop.gate", gate("open", json!(["b.md"]))).await;
        let (status, _) = request(&app, "POST", &format!("{uri}/accept"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = request(&app, "POST", "/v1/sessions", spawn(3, "k3")).await;
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
