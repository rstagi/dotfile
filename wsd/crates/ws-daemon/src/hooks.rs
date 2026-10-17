//! Durable agent callbacks and ordered tmux state updates.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Sessions},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ws_core::ids::{SessionId, Ulid};

const TMUX_DEADLINE: Duration = Duration::from_millis(100);
pub(crate) static INGEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    #[serde(default)]
    pub notify: Option<crate::notify::Config>,
}

#[derive(Clone, Debug, Default)]
pub enum StopDecision {
    #[default]
    Accept,
    Block(String),
}

#[derive(Default)]
struct Window {
    owned: bool,
    id: String,
    seq: u64,
    state: String,
    turn: String,
    main: String,
    session: String,
    pane: String,
    focused: bool,
    subagents: BTreeSet<String>,
    notification: crate::notify::Target,
}

struct Reduction {
    changes: Vec<(String, Option<String>)>,
    accepted: bool,
    state: String,
    event_type: Option<&'static str>,
}

/// Records a callback before reduction. Delivery supplies the accepted or blocked Stop decision.
pub async fn ingest(
    store: std::sync::Arc<std::sync::Mutex<Sessions>>,
    callback: Callback,
    decision: StopDecision,
) -> crate::sessions::Result<Value> {
    let _guard = INGEST_LOCK.lock().await;
    validate(&callback)?;
    // Persist the original callback before reading or changing any window state.
    let raw = callback.clone();
    let initial_reply = stop_reply(&decision);
    let captured = api::call(store.clone(), move |sessions| {
        let mut record = body(&raw);
        record["stop_reply"] = initial_reply;
        sessions.append_callback(&raw, "hook.received", record)
    })
    .await
    .map_err(|error| error.0)?;
    // A transport retry must retain a Stop decision that the daemon already recorded.
    let decision = if captured["stop_reply"]["decision"] == "block" {
        StopDecision::Block(
            captured["stop_reply"]["reason"]
                .as_str()
                .unwrap_or("")
                .into(),
        )
    } else {
        StopDecision::Accept
    };
    let mut window = window(&callback).await?;
    if let Some(window) = window.as_mut().filter(|window| window.session.is_empty()) {
        let session = callback.session_id.to_string();
        let id = window.id.clone();
        window.owned = api::call(store.clone(), move |sessions| {
            Ok(sessions.registry.workspaces()?.iter().any(|workspace| {
                workspace.body["windows"].as_array().is_some_and(|windows| {
                    windows
                        .iter()
                        .any(|window| window["window_id"] == id && window["session_id"] == session)
                })
            }))
        })
        .await
        .map_err(|error| error.0)?;
    }
    let reduction = reduce(&callback, window.as_ref(), &decision);
    if let Some(event_type) = reduction.event_type {
        let callback = callback.clone();
        let mut event = body(&callback);
        event.as_object_mut().unwrap().remove("payload");
        event["state_accepted"] = json!(reduction.accepted);
        event["window_state"] = json!(reduction.state);
        event["window_id"] = json!(window.as_ref().map(|window| &window.id));
        event["stop_reply"] = stop_reply(&decision);
        api::call(store, move |sessions| {
            sessions.append_callback(&callback, event_type, event)
        })
        .await
        .map_err(|error| error.0)?;
    }
    if let Some(window) = window {
        apply(&callback, &window.id, &reduction.changes).await?;
        if reduction.accepted {
            crate::notify::transition(
                &callback,
                &window.notification,
                &window.state,
                &reduction.state,
            )
            .await;
        }
    }
    Ok(stop_reply(&decision))
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/hooks/ingest", post(record))
}

pub fn receipt_time_us() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_micros()
        .try_into()
        .map_err(io::Error::other)
}

/// Writes a callback atomically for replay after a failed transport.
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
                .map_err(io::Error::other)?;
            fs::remove_file(&path)
        }
        .await;
        if let Err(error) = result {
            eprintln!("Cannot replay hook {}: {error}", path.display());
        }
    }
    Ok(())
}

async fn record(
    State(store): State<Store>,
    Json(callback): Json<Callback>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // The hook client has a short deadline. Its disconnect must not cancel ingest.
    let reply = tokio::spawn(ingest(store, callback, StopDecision::Accept))
        .await
        .map_err(|error| ApiError(io::Error::other(error).into()))?
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(reply)))
}

fn stop_reply(decision: &StopDecision) -> Value {
    match decision {
        StopDecision::Accept => json!({}),
        StopDecision::Block(reason) => json!({"decision":"block", "reason":reason}),
    }
}

impl Sessions {
    fn append_callback(
        &mut self,
        callback: &Callback,
        event_type: &str,
        body: Value,
    ) -> crate::sessions::Result<Value> {
        self.record_call(None, "hook", None)?;
        self.get_record(&callback.session_id.to_string())?;
        if let Some(entry) = self.registry.event_for_callback(
            callback.session_id,
            event_type,
            &callback.callback_id.to_string(),
        )? {
            if event_type == "hook.received"
                && [
                    "payload",
                    "state",
                    "pane",
                    "observed_at_us",
                    "tmux_socket",
                    "tmux_path",
                ]
                .iter()
                .any(|key| entry.body[key] != body[key])
            {
                return Err(Error::InvalidRequest(
                    "The callback id has different content.",
                ));
            }
            return Ok(entry.body);
        }
        self.append(callback.session_id, event_type, body.clone())?;
        Ok(body)
    }
}

async fn window(callback: &Callback) -> io::Result<Option<Window>> {
    let Some(pane) = &callback.pane else {
        return Ok(None);
    };
    if callback.tmux_socket.is_none() && callback.tmux_path.is_none() {
        return Ok(None);
    }
    let snapshot = tmux(callback, &["display-message", "-p", "-t", pane,
        "#{window_id}|#{@ws_seq}|#{@ws_state}|#{@ws_turn}|#{@ws_main}|#{@ws_session}|#{@ws_pane}|#{&&:#{window_active},#{session_attached}}\u{1f}#{session_id}\u{1f}#{@ws_path}\u{1f}#{@ws_agent}",
        ";", "show-options", "-w", "-t", pane]).await?;
    let mut lines = snapshot.lines();
    let metadata: Vec<_> = lines.next().unwrap_or("").split('\u{1f}').collect();
    let fields: Vec<_> = metadata[0].split('|').collect();
    if fields.len() != 8 || metadata.len() != 4 || !valid_id(fields[0], '@') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The hook window response is invalid.",
        ));
    }
    Ok(Some(Window {
        owned: true,
        id: fields[0].into(),
        seq: fields[1].parse().unwrap_or(0),
        state: fields[2].into(),
        turn: fields[3].into(),
        main: fields[4].into(),
        session: fields[5].into(),
        pane: fields[6].into(),
        focused: fields[7] == "1",
        notification: crate::notify::Target {
            window: fields[0].into(),
            session: metadata[1].into(),
            path: metadata[2].into(),
            agent: metadata[3].into(),
            focused: fields[7] == "1",
        },
        subagents: lines
            .filter_map(|line| line.split_once(' ').map(|(key, _)| key.to_owned()))
            .filter(|key| key.starts_with("@ws_sub_"))
            .collect(),
    }))
}

fn reduce(callback: &Callback, window: Option<&Window>, decision: &StopDecision) -> Reduction {
    let mut result = Reduction {
        changes: vec![],
        accepted: false,
        state: String::new(),
        event_type: event_type(callback, decision),
    };
    let mut event = callback.state.as_str();
    if event == "idle" {
        match event_name(callback) {
            "Stop" | "stop" => event = "stop",
            "StopCancelled" | "stop_cancelled" | "Interrupt" => event = "cancel",
            _ => {}
        }
    }
    if event == "stop"
        && matches!(
            callback.payload["reason"].as_str(),
            Some("channel_closed" | "shutdown")
        )
    {
        result.event_type = None;
        return result;
    }
    let Some(window) = window else {
        return result;
    };
    result.state = window.state.clone();
    if !window.owned
        || (!window.session.is_empty() && window.session != callback.session_id.to_string())
        || (!window.pane.is_empty() && Some(window.pane.as_str()) != callback.pane.as_deref())
    {
        result.event_type = None;
        return result;
    }
    let agent = callback.payload["agent_id"].as_str().unwrap_or("");
    let agent: String = agent
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let sub = format!("@ws_sub_{agent}");
    let mut subagents = window.subagents.clone();
    if event == "subagent-start" && !agent.is_empty() {
        subagents.insert(sub.clone());
        result.changes.push((sub.clone(), Some("1".into())));
    } else if event == "subagent-stop" && !agent.is_empty() {
        subagents.remove(&sub);
        result.changes.push((sub, None));
    }
    if callback.observed_at_us <= window.seq {
        if event == "stop" {
            result.event_type = None;
        }
        return result;
    }
    let turn = callback.payload["promptId"]
        .as_str()
        .or_else(|| callback.payload["prompt_id"].as_str())
        .or_else(|| callback.payload["turn_id"].as_str())
        .unwrap_or("");
    if event == "cancel" {
        if turn.is_empty() || turn != window.turn {
            result.event_type = None;
            return result;
        }
        event = "idle";
        result.event_type = Some("turn.cancelled");
    }
    let state = match event {
        "stop" | "idle" => {
            if event == "stop" && matches!(decision, StopDecision::Block(_)) {
                result
                    .changes
                    .push(("@ws_main".into(), Some("working".into())));
                "working"
            } else {
                result.changes.push(("@ws_turn".into(), None));
                result
                    .changes
                    .push(("@ws_main".into(), Some("idle".into())));
                if subagents.is_empty() {
                    "idle"
                } else {
                    "working"
                }
            }
        }
        "working" => {
            if agent.is_empty() {
                result
                    .changes
                    .push(("@ws_main".into(), Some("working".into())));
                if !turn.is_empty()
                    || matches!(
                        event_name(callback),
                        "UserPromptSubmit" | "user_prompt_submit"
                    )
                {
                    result.changes.push(("@ws_turn".into(), Some(turn.into())));
                }
            }
            "working"
        }
        "subagent-start" => "working",
        "subagent-stop" => {
            if subagents.is_empty() && window.main == "idle" {
                "idle"
            } else if window.state.is_empty() {
                "working"
            } else {
                &window.state
            }
        }
        _ => event,
    };
    if window.state == "working" && state == "idle" && !window.focused {
        result.changes.push(("@ws_unread".into(), Some("1".into())));
    }
    result
        .changes
        .push(("@ws_seq".into(), Some(callback.observed_at_us.to_string())));
    result
        .changes
        .push(("@ws_state".into(), Some(state.into())));
    result.accepted = true;
    result.state = state.into();
    result
}

async fn apply(
    callback: &Callback,
    window: &str,
    changes: &[(String, Option<String>)],
) -> io::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut args = Vec::<String>::new();
    for (option, value) in changes {
        if !args.is_empty() {
            args.push(";".into());
        }
        args.extend([
            "set-option".into(),
            if value.is_some() { "-w" } else { "-wu" }.into(),
            "-t".into(),
            window.into(),
            option.clone(),
        ]);
        if let Some(value) = value {
            args.push(value.clone());
        }
    }
    tmux(
        callback,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .await?;
    Ok(())
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

fn event_type(callback: &Callback, decision: &StopDecision) -> Option<&'static str> {
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
        "hook_event_name": event_name(callback), "pane": callback.pane,
        "tmux_socket": callback.tmux_socket, "tmux_path": callback.tmux_path});
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
async fn tmux(callback: &Callback, args: &[&str]) -> io::Result<String> {
    tmux_server(
        callback.tmux_socket.as_deref(),
        callback.tmux_path.as_deref(),
        args,
    )
    .await
}

pub(crate) async fn tmux_server(
    socket: Option<&str>,
    path: Option<&str>,
    args: &[&str],
) -> io::Result<String> {
    let mut command = tokio::process::Command::new("tmux");
    if let Some(socket) = socket {
        command.args(["-L", socket]);
    } else if let Some(path) = path {
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
