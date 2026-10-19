//! Interactive agent registration and generation-scoped exit handling.

use super::{AgentSession, Mode, Prepared, prepare, slots};
use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
    tmux::{PaneStatus, Tmux},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use ws_core::{ids::SessionId, log::SessionLog, registry::Session};

// Removing a sender releases every request that waits for this launch to commit.
static PENDING: LazyLock<Mutex<HashMap<SessionId, watch::Sender<()>>>> =
    LazyLock::new(Default::default);
// The completion signal controls readiness. This deadline only bounds a stalled commit.
const COMMIT_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct PendingLaunch(SessionId);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunAgent {
    pub target: String,
    pub pane: String,
    pub agent: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRequest {
    pub id: String,
    pub pane: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExitRequest {
    pub id: String,
    pub pane: String,
    pub code: i32,
    pub user_tab: bool,
}

#[derive(Deserialize, Serialize)]
pub struct Launch {
    pub prepared: Prepared,
    pub directory: PathBuf,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/agents/run", post(run))
        .route("/v1/agents/launch", post(launch))
        .route("/v1/agents/exit", post(exited))
}

async fn run(
    State(store): State<Store>,
    Json(request): Json<RunAgent>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.run_agent(request)).await?,
    ))
}

async fn launch(
    State(store): State<Store>,
    Json(request): Json<LaunchRequest>,
) -> std::result::Result<Json<Launch>, ApiError> {
    let (request, pending) = api::call(store.clone(), move |sessions| {
        let session = sessions.get_record(&request.id)?;
        let pending = PENDING
            .lock()
            .map_err(|_| Error::InvalidRequest("Cannot lock the pending agent launches."))?
            .get(&session.id)
            .map(watch::Sender::subscribe);
        Ok((request, pending))
    })
    .await?;
    if let Some(path) = env::var_os("WS_TEST_AGENT_LAUNCH_OBSERVED") {
        std::fs::write(path, b"requested").map_err(|error| ApiError(Error::Io(error)))?;
    }
    if let Some(mut pending) = pending {
        // The commit needs the store lock. Wait without that lock, then check pane ownership.
        tokio::time::timeout(COMMIT_TIMEOUT, pending.changed())
            .await
            .map_err(|_| {
                ApiError(Error::InvalidRequest(
                    "The agent launch did not commit before the deadline.",
                ))
            })?
            .ok();
    }
    Ok(Json(
        api::call(store, move |sessions| sessions.prepare_agent(request)).await?,
    ))
}

async fn exited(
    State(store): State<Store>,
    Json(request): Json<ExitRequest>,
) -> std::result::Result<Json<Value>, ApiError> {
    api::call(store, move |sessions| sessions.agent_exited(request)).await?;
    Ok(Json(json!({})))
}

impl PendingLaunch {
    pub(crate) fn new(id: SessionId) -> Result<Self> {
        let (completion, _) = watch::channel(());
        PENDING
            .lock()
            .map_err(|_| Error::InvalidRequest("Cannot lock the pending agent launches."))?
            .insert(id, completion);
        Ok(Self(id))
    }
}

impl Drop for PendingLaunch {
    fn drop(&mut self) {
        if let Ok(mut pending) = PENDING.lock() {
            pending.remove(&self.0);
        }
    }
}

impl Sessions {
    pub(crate) fn reopen_for_message(
        &mut self,
        session: &Session,
        caller: Option<SessionId>,
    ) -> Result<()> {
        let directory = self.agent_directory(session)?;
        let old_windows = self.session_windows(session)?;
        let tmux = Tmux::from_env();
        if !self.process_exited(session.id)? {
            for window in &old_windows {
                let Some(pane) = tmux.window_option(window, "@ws_pane").ok().flatten() else {
                    continue;
                };
                if self.owns_agent_window(session, &pane)?
                    && tmux.pane_status(&pane)? == PaneStatus::Running
                {
                    // Completion ends the task. The existing agent still owns its conversation.
                    self.transition(
                        &session.id.to_string(),
                        ws_core::state::SessionState::Reopened,
                        caller,
                    )?;
                    self.transition(
                        &session.id.to_string(),
                        ws_core::state::SessionState::Running,
                        caller,
                    )?;
                    return Ok(());
                }
            }
        }
        let id = session.id.to_string();
        let tmux_session = session.workspace_id.as_deref().unwrap_or(&id);
        let launched = self.start_workspace_session(
            tmux_session,
            &directory,
            session.body["agent"].as_str().unwrap_or("claude"),
            caller,
            Some(session),
            None,
        )?;
        for window in old_windows {
            if launched["window_id"] == window
                || tmux
                    .window_option(&window, "@ws_session")
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some(&session.id.to_string())
            {
                continue;
            }
            // A wrapper can retain its pane after the agent exits.
            for key in [
                "@ws_session",
                "@ws_pane",
                "@ws_slot",
                "@ws_agent",
                "@ws_state",
                "@ws_main",
            ] {
                let _ = tmux.unset_window_option(&window, key);
            }
        }
        Ok(())
    }

    fn run_agent(&mut self, request: RunAgent) -> Result<Value> {
        self.record_call(
            request.parent_session_id,
            "agent.run",
            Some(&request.target),
        )?;
        if !["claude", "codex", "grok"].contains(&request.agent.as_str()) {
            return Err(Error::InvalidRequest(
                "agent must be one of: claude, codex, grok",
            ));
        }
        let (name, wt) = crate::workspaces::workspace_path(&request.target)?;
        let tmux = Tmux::from_env();
        let window = tmux.window_for_pane(&request.pane)?;
        if tmux
            .window_option(&window, "@ws_slot")?
            .is_some_and(|s| !s.is_empty())
        {
            return Err(Error::Io(std::io::Error::other(format!(
                "{}: this tab already runs a ws session",
                request.agent
            ))));
        }
        let pane_directory = tmux.execute(&[
            "display-message",
            "-p",
            "-t",
            &request.pane,
            "#{pane_current_path}",
        ])?;
        if !Path::new(&pane_directory)
            .canonicalize()
            .is_ok_and(|path| path.starts_with(&wt))
        {
            return Err(Error::Io(std::io::Error::other(format!(
                "{}: not inside a workspace",
                request.agent
            ))));
        }
        self.start_workspace_session(
            &name,
            &wt,
            &request.agent,
            request.parent_session_id,
            None,
            Some(&request),
        )
    }

    fn prepare_agent(&mut self, request: LaunchRequest) -> Result<Launch> {
        let session = self.get(&request.id, None)?;
        let directory = self.agent_directory(&session)?;
        if !self.owns_agent_window(&session, &request.pane)? {
            return Err(Error::InvalidRequest(
                "This pane does not own the agent session.",
            ));
        }
        let home = PathBuf::from(env::var_os("HOME").ok_or(Error::InvalidRequest(
            "Set HOME to locate agent configuration files.",
        ))?);
        let ws_home = env::var_os("WS_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".ws"));
        let arguments = session.body["arguments"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let context = AgentSession {
            claude_config_dir: env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            agent_program: None,
            state: self.state.clone(),
            parent_session_id: session.body["parent_session_id"]
                .as_str()
                .and_then(|id| id.parse().ok()),
            project: session.workspace_id.clone().unwrap_or_default(),
            id: session.id.to_string(),
            home,
            grok_home: env::var_os("GROK_HOME").map(PathBuf::from),
            codex_home: env::var_os("CODEX_HOME").map(PathBuf::from),
            ws_home,
            ws_bin: env::current_exe()?,
            spawned: session.body["spawned"].as_bool().unwrap_or(false),
            prompt: if self
                .registry
                .latest_event(session.id, "agent.prompt_consumed")?
                .is_none()
            {
                session.body["prompt"].as_str().map(str::to_owned)
            } else {
                None
            },
            arguments,
        };
        let resume = self.agent_resume_id(&directory, &session)?;
        let mut prepared = prepare(
            session.body["agent"].as_str().unwrap_or("claude"),
            Mode::Interactive,
            &context,
            resume.as_deref(),
        )?;
        prepared.env.insert(
            "WS_WORKSPACE".into(),
            session.workspace_id.clone().unwrap_or_default(),
        );
        prepared.env.insert(
            "WS_BIN".into(),
            context.ws_bin.to_string_lossy().into_owned(),
        );
        prepared.write_files()?;
        if context.prompt.is_some() {
            self.append(session.id, "agent.prompt_consumed", json!({}))?;
        }
        Ok(Launch {
            prepared,
            directory,
        })
    }

    fn agent_resume_id(&self, directory: &Path, session: &Session) -> Result<Option<String>> {
        // A clean exit removes the slot row. The hook log retains the conversation id.
        let entries = SessionLog::new(&self.state, session.id, session.device_id)
            .read()?
            .entries;
        let resume = entries.iter().rev().find_map(|entry| {
            if entry.event_type != "hook.received"
                || !entry.body["payload"]["agent_id"].is_null()
                || entry.body["payload"]["hook_event_name"]
                    .as_str()
                    .is_some_and(|event| event.starts_with("Subagent"))
            {
                return None;
            }
            entry.body["payload"]["session_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        });
        if resume.is_some() {
            return Ok(resume);
        }
        Ok(
            slots::resume(&self.state, directory, &session.id.to_string())?
                .or_else(|| session.body["resume_id"].as_str().map(str::to_owned)),
        )
    }

    fn agent_exited(&mut self, request: ExitRequest) -> Result<()> {
        let session = self.get(&request.id, None)?;
        if !self.owns_agent_window(&session, &request.pane)? {
            return Ok(());
        }
        if !self.process_exited(session.id)? {
            self.append(
                session.id,
                "exit",
                json!({"code":request.code, "pane":request.pane}),
            )?;
        }
        if request.code == 0 {
            slots::remove(&self.state, &self.agent_directory(&session)?, &request.id)?;
            if !["finished", "failed"].contains(&session.state.as_str()) {
                self.transition(&request.id, ws_core::state::SessionState::Finished, None)?;
            }
        }
        if request.user_tab || request.code == 0 {
            let tmux = Tmux::from_env();
            let window = tmux.window_for_pane(&request.pane)?;
            for key in [
                "@ws_slot",
                "@ws_agent",
                "@ws_state",
                "@ws_session",
                "@ws_pane",
                "automatic-rename",
            ] {
                tmux.unset_window_option(&window, key)?;
            }
        }
        Ok(())
    }

    pub(crate) fn agent_directory(&self, session: &Session) -> Result<PathBuf> {
        if let Some(cwd) = &session.cwd {
            return Ok(PathBuf::from(cwd));
        }
        self.registry
            .workspaces()?
            .into_iter()
            .find(|w| Some(w.id.as_str()) == session.workspace_id.as_deref())
            .and_then(|w| w.body["path"].as_str().map(PathBuf::from))
            .ok_or(Error::InvalidRequest("The agent session has no directory."))
    }

    fn owns_agent_window(&self, session: &Session, pane: &str) -> Result<bool> {
        if !pane
            .strip_prefix('%')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(Error::InvalidRequest("Use a tmux pane id such as %1."));
        }
        let tmux = Tmux::from_env();
        let window = tmux.window_for_pane(pane)?;
        let id = session.id.to_string();
        // zsh-launched windows keep the imported session's numeric alias as their tag.
        let alias = session.body["alias"]
            .as_i64()
            .map(|alias| alias.to_string());
        let tag = tmux.window_option(&window, "@ws_session")?;
        if !(tag.as_deref() == Some(&id) || tag.is_some() && tag == alias)
            || tmux.window_option(&window, "@ws_pane")?.as_deref() != Some(pane)
        {
            return Ok(false);
        }
        Ok(self.session_windows(session)?.contains(&window))
    }
}
