use crate::{Result, client};
use serde_json::{Value, json};
use std::{
    env,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use ws_core::ids::Ulid;
use ws_daemon::hooks::{self, Callback};

const TRANSPORT_DEADLINE: Duration = Duration::from_millis(250);

/// Hooks always succeed. A failed transport leaves its callback in the spool.
pub async fn run(socket: &Path, state: &str) {
    let observed_at_us = hooks::receipt_time_us().unwrap_or(0);
    let mut input = String::new();
    if !io::stdin().is_terminal() {
        let _ = io::stdin().read_to_string(&mut input);
    }
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({"raw":input}));
    let event = payload["hook_event_name"]
        .as_str()
        .or_else(|| payload["hookEventName"].as_str())
        .unwrap_or("");
    let state = match (state, event) {
        ("idle", "Stop" | "stop") => "stop",
        ("idle", "StopCancelled" | "stop_cancelled" | "Interrupt") => "cancel",
        _ => state,
    };
    let reply = deliver(socket, state, payload, observed_at_us)
        .await
        .unwrap_or_else(|_| json!({}));
    if state == "stop" {
        println!("{reply}");
    }
}

async fn deliver(socket: &Path, state: &str, payload: Value, observed_at_us: u64) -> Result<Value> {
    let session = env::var("WS_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| env::var("WS_SESSION").ok().filter(|id| !id.is_empty()));
    let Some(session) = session.and_then(|id| id.parse().ok()) else {
        return Ok(json!({}));
    };
    if ![
        "working",
        "idle",
        "stop",
        "cancel",
        "waiting",
        "subagent-start",
        "subagent-stop",
    ]
    .contains(&state)
    {
        return Ok(json!({}));
    }
    let callback = Callback {
        callback_id: Ulid::new(),
        session_id: session,
        observed_at_us,
        state: state.into(),
        pane: env::var("TMUX_PANE").ok(),
        tmux_socket: env::var("WS_TMUX_SOCKET")
            .ok()
            .filter(|socket| !socket.is_empty()),
        tmux_path: env::var("TMUX").ok().and_then(|value| {
            value
                .split(',')
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        }),
        payload,
    };
    if let Ok(Ok(reply)) = tokio::time::timeout(
        TRANSPORT_DEADLINE,
        client::request(
            socket,
            hyper::Method::POST,
            "/v1/hooks/ingest",
            serde_json::to_vec(&callback)?,
        ),
    )
    .await
    {
        return Ok(serde_json::from_slice(&reply).unwrap_or_else(|_| json!({})));
    }
    let state_path = env::var_os("WS_STATE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws/state")))
        .ok_or("Set HOME or WS_STATE to locate the hook spool.")?;
    hooks::spool(&state_path, &callback)?;
    Ok(json!({}))
}
