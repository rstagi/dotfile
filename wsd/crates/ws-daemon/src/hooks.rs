//! Durable agent callbacks and ordered tmux state updates.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Sessions},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ws_core::ids::{SessionId, Ulid};

const TMUX_DEADLINE: Duration = Duration::from_millis(20);
static INGEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Callback {
    pub callback_id: Ulid,
    pub session_id: SessionId,
    pub observed_at_us: u64,
    pub state: String,
    pub pane: Option<String>,
    pub tmux_socket: Option<String>,
    pub tmux_path: Option<String>,
    pub payload: Value,
}

#[derive(Clone, Debug, Default)]
pub enum StopDecision {
    #[default]
    Accept,
    Block(String),
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/hooks/ingest", post(record))
}

async fn record(
    State(store): State<Store>,
    Json(callback): Json<Callback>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let reply = ingest(store, callback, StopDecision::Accept).await?;
    Ok((StatusCode::CREATED, Json(reply)))
}

pub fn receipt_time_us() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_micros()
        .try_into()
        .map_err(io::Error::other)
}

/// Writes each callback atomically before transport. A daemon restart can replay it.
pub fn spool(state: &Path, callback: &Callback) -> io::Result<PathBuf> {
    let directory = state.join("spool");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let path = directory.join(format!(
        "{:020}-{}.json",
        callback.observed_at_us, callback.callback_id
    ));
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(callback)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    fs::File::open(&directory)?.sync_all()?;
    Ok(path)
}

pub(crate) async fn replay(store: Store, state: &Path) -> io::Result<()> {
    let directory = state.join("spool");
    let mut paths = match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    paths.sort();
    for path in paths {
        // Preserve a malformed or failed record. It must not prevent other sessions from replaying.
        let result = async {
            let callback = serde_json::from_slice(&fs::read(&path)?)?;
            ingest(store.clone(), callback, StopDecision::Accept)
                .await
                .map_err(|error| io::Error::other(error.0))?;
            fs::remove_file(&path)
        }
        .await;
        if let Err(error) = result {
            eprintln!("Cannot replay hook {}: {error}", path.display());
        }
    }
    Ok(())
}

pub(crate) async fn ingest(
    store: Store,
    callback: Callback,
    decision: StopDecision,
) -> Result<Value, ApiError> {
    let _guard = INGEST_LOCK.lock().await;
    validate(&callback).map_err(ApiError)?;
    let raw = callback.clone();
    api::call(store.clone(), move |sessions| {
        sessions.append_callback(&raw, "hook.received", body(&raw))
    })
    .await?;
    let event_type = event_type(&callback, &decision);
    if let Some(event_type) = event_type {
        let callback = callback.clone();
        api::call(store, move |sessions| {
            sessions.append_callback(&callback, event_type, body(&callback))
        })
        .await?;
    }
    Ok(match decision {
        StopDecision::Accept => json!({}),
        StopDecision::Block(reason) => json!({"decision":"block", "reason":reason}),
    })
}

impl Sessions {
    fn append_callback(
        &mut self,
        callback: &Callback,
        event_type: &str,
        body: Value,
    ) -> crate::sessions::Result<()> {
        self.record_call(None, "hook", None)?;
        self.get_record(&callback.session_id.to_string())?;
        let entries = self.session_log(callback.session_id).read()?.entries;
        if !entries.iter().any(|entry| {
            entry.event_type == event_type
                && entry.body["callback_id"] == callback.callback_id.to_string()
        }) {
            self.append(callback.session_id, event_type, body)?;
        }
        Ok(())
    }
}

fn validate(callback: &Callback) -> crate::sessions::Result<()> {
    if ![
        "working",
        "idle",
        "stop",
        "cancel",
        "waiting",
        "subagent-start",
        "subagent-stop",
    ]
    .contains(&callback.state.as_str())
    {
        return Err(Error::InvalidRequest("Use a supported hook state."));
    }
    if callback
        .pane
        .as_ref()
        .is_some_and(|pane| !valid_id(pane, '%'))
    {
        return Err(Error::InvalidRequest("Use a tmux pane id such as %1."));
    }
    Ok(())
}

fn valid_id(id: &str, prefix: char) -> bool {
    id.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn event_name(callback: &Callback) -> &str {
    callback.payload["hook_event_name"]
        .as_str()
        .or_else(|| callback.payload["hookEventName"].as_str())
        .unwrap_or("")
}

fn event_type<'a>(callback: &Callback, decision: &StopDecision) -> Option<&'a str> {
    Some(match event_name(callback) {
        "UserPromptSubmit" | "user_prompt_submit" => "turn.started",
        "PreToolUse" | "pre_tool_use" => "tool.pre",
        "PostToolUse" | "post_tool_use" => "tool.post",
        "PermissionRequest" | "Notification" | "permission_request" | "notification" => {
            "agent.waiting"
        }
        "SubagentStart" | "subagent_start" => "subagent.started",
        "SubagentStop" | "subagent_stop" => "subagent.stopped",
        "Stop" | "stop" => {
            if matches!(decision, StopDecision::Block(_)) {
                "turn.continued"
            } else {
                "turn.ended"
            }
        }
        _ => match callback.state.as_str() {
            "stop" => {
                if matches!(decision, StopDecision::Block(_)) {
                    "turn.continued"
                } else {
                    "turn.ended"
                }
            }
            "waiting" => "agent.waiting",
            "subagent-start" => "subagent.started",
            "subagent-stop" => "subagent.stopped",
            _ => return None,
        },
    })
}

fn body(callback: &Callback) -> Value {
    let mut body = json!({"callback_id": callback.callback_id, "observed_at_us": callback.observed_at_us,
        "seq": callback.observed_at_us, "state": callback.state, "payload": callback.payload,
        "hook_event_name": event_name(callback), "pane": callback.pane});
    for (target, keys) in [
        ("prompt_id", &["prompt_id", "promptId"][..]),
        ("turn_id", &["turn_id"][..]),
        ("tool_use_id", &["tool_use_id", "toolUseId", "call_id"][..]),
        ("tool_name", &["tool_name", "toolName"][..]),
        ("agent_id", &["agent_id"][..]),
        ("agent_duration_ms", &["duration_ms"][..]),
    ] {
        if let Some(value) = keys.iter().find_map(|key| callback.payload.get(key)) {
            body[target] = value.clone();
        }
    }
    body
}

/// Executes one tmux command with a deadline. Timeout cancels and kills the child.
pub async fn tmux(callback: &Callback, args: &[&str]) -> io::Result<String> {
    let mut command = tokio::process::Command::new("tmux");
    if let Some(socket) = &callback.tmux_socket {
        command.args(["-L", socket]);
    } else if let Some(path) = &callback.tmux_path {
        command.args(["-S", path]);
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "The hook has no tmux server.",
        ));
    }
    let output = tokio::time::timeout(
        TMUX_DEADLINE,
        command.args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "The hook tmux command exceeded its deadline.",
        )
    })??;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim_end_matches('\n').to_owned())
        .map_err(io::Error::other)
}
