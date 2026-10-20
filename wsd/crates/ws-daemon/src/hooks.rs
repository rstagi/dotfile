//! Durable agent callbacks and ordered tmux state updates.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Sessions},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ws_core::ids::{SessionId, Ulid};

/// Total transport budget for Stop and exit. Generated hooks add a 10 s parent margin.
pub const STOP_CLI_BUDGET: Duration = Duration::from_secs(20);

// Bound each tmux call independently of the hook client.
const TMUX_DEADLINE: Duration = Duration::from_secs(2);
// Recovery of an unconfirmed block retries this many times when a tmux call times out.
const RECOVERY_ATTEMPTS: u32 = 3;
const SPOOL_INTERVAL: Duration = Duration::from_secs(1);
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
pub(crate) struct HookState {
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
    ingest_callback(store, callback, decision, true).await
}

/// Drains callbacks concurrently with the socket server until daemon shutdown.
pub(crate) async fn drain(store: Store, state: &Path) -> io::Result<()> {
    let mut pending = VecDeque::new();
    let mut seen = BTreeSet::new();
    let mut scan_at = tokio::time::Instant::now();
    loop {
        if pending.is_empty() {
            tokio::time::sleep_until(scan_at).await;
            // Retry failed files once the current queue completes.
            seen.clear();
        }
        if tokio::time::Instant::now() >= scan_at {
            scan_at = tokio::time::Instant::now() + SPOOL_INTERVAL;
            if let Err(error) = api::call(store.clone(), crate::bus::delivery::replay).await {
                eprintln!("Cannot drain the delivery spool: {}", error.0);
            }
            match spool_paths(state) {
                Ok(paths) => {
                    // Preserve each scan's order. New arrivals precede the existing backlog.
                    let fresh: Vec<_> = paths
                        .into_iter()
                        .filter(|path| seen.insert(path.clone()))
                        .collect();
                    for path in fresh.into_iter().rev() {
                        pending.push_front(path);
                    }
                }
                Err(error) => eprintln!("Cannot scan the hook spool: {error}"),
            }
        }
        if let Some(path) = pending.pop_front() {
            replay_file(store.clone(), &path).await;
        }
    }
}

async fn ingest_callback(
    store: Store,
    callback: Callback,
    decision: StopDecision,
    live: bool,
) -> crate::sessions::Result<Value> {
    let _guard = INGEST_LOCK.lock().await;
    validate(&callback)?;
    // Persist the original callback before reading or changing any window state.
    let raw = callback.clone();
    let initial_reply = stop_reply(&decision);
    let captured = api::call(store.clone(), move |sessions| {
        crate::bus::delivery::replay(sessions)?;
        sessions.record_call(None, "hook", None)?;
        if sessions.get_record(&raw.session_id.to_string())?.device_id != sessions.device_id {
            return Ok(None);
        }
        let mut record = body(&raw);
        record["stop_reply"] = initial_reply;
        let mut captured = sessions.append_callback(&raw, "hook.received", record)?;
        if raw.state == "quota" {
            let _ = sessions
                .quota_requests
                .sender()
                .send(crate::quota::Trigger::Boundary(raw.session_id));
        }
        if let Some(decision) = sessions.delivery_decision(raw.session_id, raw.callback_id)? {
            captured["stop_reply"] = stop_reply(&decision);
        }
        Ok(Some(captured))
    })
    .await
    .map_err(|error| error.0)?;
    let Some(captured) = captured else {
        return Ok(json!({}));
    };
    if callback.state == "quota" {
        return Ok(json!({}));
    }
    // A transport retry must retain a Stop decision that the daemon already recorded.
    let mut decision = if captured["stop_reply"]["decision"] == "block" {
        StopDecision::Block(
            captured["stop_reply"]["reason"]
                .as_str()
                .unwrap_or("")
                .into(),
        )
    } else {
        StopDecision::Accept
    };
    let mut window = match window(&callback).await {
        Ok(window) => window,
        Err(error) if event_type(&callback, &decision) == Some("exit") => {
            // Exit recording must not depend on a pane that tmux can already have removed.
            eprintln!("Cannot read the exiting agent window: {error}");
            None
        }
        // A replayed record whose pane is gone stays undeliverable. Record it without the window.
        Err(_) if !live => None,
        Err(error) => return Err(error.into()),
    };
    if callback.pane.is_none() {
        let callback = callback.clone();
        window = Some(
            api::call(store.clone(), move |sessions| {
                sessions.headless_hook_state(&callback)
            })
            .await
            .map_err(|error| error.0)?,
        );
    }
    if let Some(window) = window.as_mut().filter(|window| window.session.is_empty()) {
        let session = callback.session_id;
        let id = window.id.clone();
        window.owned = api::call(store.clone(), move |sessions| {
            let session = sessions.get_record(&session.to_string())?;
            Ok(sessions.session_windows(&session)?.contains(&id))
        })
        .await
        .map_err(|error| error.0)?;
    }
    if !window
        .as_ref()
        .is_some_and(|window| owns_window(&callback, window))
    {
        decision = StopDecision::Accept;
    }
    if window
        .as_ref()
        .is_some_and(|window| owns_window(&callback, window))
    {
        let raw = callback.clone();
        let id = window.as_ref().unwrap().id.clone();
        api::call(store.clone(), move |sessions| {
            sessions.acknowledge_paste(&raw, &id)
        })
        .await
        .map_err(|error| error.0)?;
    }
    let mut reduction = reduce(&callback, window.as_ref(), &decision);
    if reduction.accepted
        && matches!(reduction.event_type, Some("turn.ended" | "turn.cancelled"))
        && matches!(decision, StopDecision::Accept)
    {
        let raw = callback.clone();
        let inject = live && reduction.event_type == Some("turn.ended");
        decision = api::call(store.clone(), move |sessions| {
            sessions.stop_delivery(&raw, inject)
        })
        .await
        .map_err(|error| error.0)?;
        reduction = reduce(&callback, window.as_ref(), &decision);
    }
    if let Some(event_type) = reduction.event_type {
        let callback = callback.clone();
        let mut event = body(&callback);
        event.as_object_mut().unwrap().remove("payload");
        event["state_accepted"] = json!(reduction.accepted);
        event["window_state"] = json!(reduction.state);
        event["window_id"] = json!(
            window
                .as_ref()
                .filter(|window| !window.id.is_empty())
                .map(|window| &window.id)
        );
        event["stop_reply"] = stop_reply(&decision);
        if event_type == "exit" {
            event["code"] = callback
                .payload
                .get("exit_code")
                .or_else(|| callback.payload.get("code"))
                .cloned()
                .unwrap_or(Value::Null);
        }
        api::call(store.clone(), move |sessions| {
            sessions.append_callback(&callback, event_type, event)
        })
        .await
        .map_err(|error| error.0)?;
    }
    if callback.pane.is_none() {
        let id = callback.session_id;
        if let Some(mut state) = window {
            apply_headless(&mut state, &reduction.changes);
            api::call(store.clone(), move |sessions| {
                sessions.headless_hooks.insert(id, state);
                Ok(())
            })
            .await
            .map_err(|error| error.0)?;
        }
    } else if let Some(window) = window {
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
    let reply = stop_reply(&decision);
    api::call(store, move |sessions| {
        sessions.delivery_reply(callback.session_id, callback.callback_id, reply)
    })
    .await
    .map_err(|error| error.0)
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

#[cfg(test)]
pub(crate) async fn replay(store: Store, state: &Path) -> io::Result<()> {
    api::call(store.clone(), crate::bus::delivery::replay)
        .await
        .map_err(|error| io::Error::other(error.0))?;
    for path in spool_paths(state)? {
        replay_file(store.clone(), &path).await;
    }
    Ok(())
}

fn spool_paths(state: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = match fs::read_dir(state.join("spool")) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    paths.sort();
    Ok(paths)
}

async fn replay_file(store: Store, path: &Path) {
    // Preserve malformed or failed records without blocking another session.
    let result = async {
        let callback = serde_json::from_slice(&fs::read(path)?)?;
        ingest_callback(store, callback, StopDecision::Accept, false)
            .await
            .map_err(io::Error::other)?;
        fs::remove_file(path)
    }
    .await;
    if let Err(error) = result {
        eprintln!("Cannot replay hook {}: {error}", path.display());
    }
}

async fn record(
    State(store): State<Store>,
    Json(callback): Json<Callback>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // The hook client has a short deadline. Its disconnect must not cancel ingest.
    let reply = tokio::spawn(async move {
        let reply = ingest(store.clone(), callback.clone(), StopDecision::Accept).await?;
        if reply["decision"] == "block" {
            // Recovery must survive cancellation of the HTTP handler as well as ingest.
            tokio::spawn(async move {
                tokio::time::sleep(STOP_CLI_BUDGET).await;
                recover_unconfirmed_with_retries(store, callback).await;
            });
        }
        Ok::<_, Error>(reply)
    })
    .await
    .map_err(|error| ApiError(io::Error::other(error).into()))?
    .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(reply)))
}

/// A slow tmux (a loaded machine) must not leave the window at `working` for good.
async fn recover_unconfirmed_with_retries(store: Store, callback: Callback) {
    for attempt in 1..=RECOVERY_ATTEMPTS {
        match recover_unconfirmed(store.clone(), callback.clone()).await {
            Ok(()) => return,
            Err(error) if attempt < RECOVERY_ATTEMPTS && is_tmux_timeout(&error) => {
                eprintln!("Recovery attempt {attempt} of the unconfirmed hook timed out: {error}");
                tokio::time::sleep(TMUX_DEADLINE).await;
            }
            Err(error) => {
                eprintln!("Cannot recover the unconfirmed hook: {error}");
                return;
            }
        }
    }
}

fn is_tmux_timeout(error: &crate::sessions::Error) -> bool {
    matches!(error, crate::sessions::Error::Io(error) if error.kind() == io::ErrorKind::TimedOut)
}

async fn recover_unconfirmed(store: Store, callback: Callback) -> crate::sessions::Result<()> {
    let _guard = INGEST_LOCK.lock().await;
    let raw = callback.clone();
    let pending = api::call(store.clone(), move |sessions| {
        sessions.unconfirmed_delivery(&raw)
    })
    .await
    .map_err(|error| error.0)?;
    if !pending {
        return Ok(());
    }
    let Some(window) = window(&callback).await? else {
        return Ok(());
    };
    if !owns_window(&callback, &window) || window.seq != callback.observed_at_us {
        return Ok(());
    }
    let Some(pane) = callback.pane.as_deref() else {
        return Ok(());
    };
    // Match submit's ownership guard and preserve any newer turn, including external tmux changes.
    let ownership = crate::bus::delivery::ownership_guard(callback.session_id, &window.id, pane);
    let guard = format!(
        "#{{&&:{ownership},#{{&&:#{{==:#{{@ws_seq}},{}}},#{{&&:#{{==:#{{@ws_state}},working}},#{{==:#{{@ws_main}},working}}}}}}}}",
        callback.observed_at_us
    );
    let state = if window.subagents.is_empty() {
        "idle"
    } else {
        "working"
    };
    let release = format!(
        "set-option -w -t {} @ws_main idle; set-option -w -t {} @ws_state {state}; set-option -wu -t {} @ws_turn; display-message -p released",
        window.id, window.id, window.id
    );
    if tmux(&callback, &["if-shell", "-F", "-t", pane, &guard, &release]).await? == "released" {
        api::call(store, move |sessions| {
            sessions.append(
                callback.session_id,
                "hook.abandoned",
                json!({"callback_id":callback.callback_id}),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| error.0)?;
    }
    Ok(())
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

    fn headless_hook_state(&mut self, callback: &Callback) -> crate::sessions::Result<HookState> {
        // Move the cached state out until reduction succeeds. A failed ingest rebuilds from receipts.
        if let Some(state) = self.headless_hooks.remove(&callback.session_id) {
            return Ok(state);
        }
        let session = self.get_record(&callback.session_id.to_string())?;
        let mut window = HookState {
            owned: true,
            session: session.id.to_string(),
            focused: true,
            state: "idle".into(),
            main: "idle".into(),
            ..HookState::default()
        };
        // Rebuild from durable receipts so restart and spool replay retain the same ordering.
        // The current receipt is already durable but has not yet passed through reduction.
        let entries = self
            .registry
            .session_events_of_type(session.id, "hook.received")?;
        for entry in entries {
            if entry.body["callback_id"] == callback.callback_id.to_string()
                || !entry.body["pane"].is_null()
            {
                continue;
            }
            let decision = if let Some(decision) = self.delivery_decision(
                session.id,
                entry.body["callback_id"]
                    .as_str()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| Error::InvalidRequest("The callback id is invalid."))?,
            )? {
                decision
            } else if entry.body["stop_reply"]["decision"] == "block" {
                StopDecision::Block(
                    entry.body["stop_reply"]["reason"]
                        .as_str()
                        .unwrap_or("")
                        .into(),
                )
            } else {
                StopDecision::Accept
            };
            let mut body = entry.body;
            body["session_id"] = json!(session.id);
            let previous: Callback = serde_json::from_value(body).map_err(Error::Json)?;
            let reduction = reduce(&previous, Some(&window), &decision);
            apply_headless(&mut window, &reduction.changes);
        }
        Ok(window)
    }
}

fn apply_headless(state: &mut HookState, changes: &[(String, Option<String>)]) {
    for (key, value) in changes {
        let value = value.clone().unwrap_or_default();
        match key.as_str() {
            "@ws_seq" => state.seq = value.parse().unwrap_or(0),
            "@ws_state" => state.state = value,
            "@ws_main" => state.main = value,
            "@ws_turn" => state.turn = value,
            _ if key.starts_with("@ws_sub_") => {
                if value.is_empty() {
                    state.subagents.remove(key);
                } else {
                    state.subagents.insert(key.clone());
                }
            }
            _ => {}
        }
    }
}

async fn window(callback: &Callback) -> io::Result<Option<HookState>> {
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
    Ok(Some(HookState {
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

fn reduce(callback: &Callback, window: Option<&HookState>, decision: &StopDecision) -> Reduction {
    let mut result = Reduction {
        changes: vec![],
        accepted: false,
        state: String::new(),
        event_type: event_type(callback, decision),
    };
    let mut event = if matches!(event_name(callback), "SessionEnd" | "session_end") {
        "exit"
    } else {
        callback.state.as_str()
    };
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
    if !owns_window(callback, window) {
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
        if event == "stop" || event == "exit" {
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
        if (!turn.is_empty() && !window.turn.is_empty() && turn != window.turn)
            || (turn.is_empty() && !matches!(window.state.as_str(), "working" | "waiting"))
        {
            result.event_type = None;
            return result;
        }
        event = "idle";
        result.event_type = Some("turn.cancelled");
    }
    let state = match event {
        "exit" => "closed",
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

fn owns_window(callback: &Callback, window: &HookState) -> bool {
    window.owned
        && (window.session.is_empty() || window.session == callback.session_id.to_string())
        && (window.pane.is_empty() || Some(window.pane.as_str()) == callback.pane.as_deref())
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
        "quota",
        "exit",
    ]
    .contains(&callback.state.as_str())
    {
        return Err(Error::InvalidRequest("Use a supported hook state."));
    }
    if event_type(callback, &StopDecision::Accept) == Some("exit") {
        let code = callback
            .payload
            .get("exit_code")
            .or_else(|| callback.payload.get("code"));
        if code.is_some_and(|code| {
            !code.is_null()
                && code
                    .as_i64()
                    .and_then(|code| i32::try_from(code).ok())
                    .is_none()
        }) {
            return Err(Error::InvalidRequest(
                "The exit code must be a 32-bit integer or null.",
            ));
        }
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
        "SessionEnd" | "session_end" => "exit",
        "UserPromptSubmit" | "user_prompt_submit" => "turn.started",
        "PreToolUse" | "pre_tool_use" => "tool.pre",
        "PostToolUse" | "post_tool_use" | "PostToolUseFailure" | "post_tool_use_failure" => {
            "tool.post"
        }
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
            "exit" => "exit",
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
    if event_type(callback, &StopDecision::Accept) == Some("tool.post") {
        body["tool_status"] = json!(crate::spans::tool_status(&callback.payload));
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
    let mut command = tokio::process::Command::from(crate::tmux::client());
    if let Some(socket) = socket {
        command.args([crate::tmux::socket_flag(socket), socket]);
    } else if let Some(path) = path {
        command.args(["-S", path]);
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "The hook has no tmux server.",
        ));
    }
    let output = tokio::time::timeout(
        tmux_deadline(),
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

// Sandboxes can extend the hang guard without changing the production deadline.
fn tmux_deadline() -> Duration {
    std::env::var("WS_TEST_HOOK_TMUX_DEADLINE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .filter(|deadline| *deadline >= TMUX_DEADLINE)
        .unwrap_or(TMUX_DEADLINE)
}
