use crate::{Result, client};
use serde_json::{Value, json};
use std::{
    env,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use ws_core::ids::Ulid;
use ws_daemon::{
    bus::delivery::{self, Receipt},
    hooks::{self, Callback},
};

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
    let (reply, receipt) = deliver(socket, state, payload, observed_at_us)
        .await
        .unwrap_or_else(|_| (json!({}), None));
    if state == "stop" {
        let written = {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_ok()
        };
        if let Some(receipt) = receipt.filter(|_| written) {
            let _ = confirm(socket, &receipt).await;
        }
    }
}

async fn deliver(
    socket: &Path,
    state: &str,
    payload: Value,
    observed_at_us: u64,
) -> Result<(Value, Option<Receipt>)> {
    let session = env::var("WS_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| env::var("WS_SESSION").ok().filter(|id| !id.is_empty()));
    let Some(session) = session.and_then(|id| id.parse().ok()) else {
        return Ok((json!({}), None));
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
        return Ok((json!({}), None));
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
        notify: Some(ws_daemon::notify::Config::from_env()),
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
        let reply: Value = serde_json::from_slice(&reply).unwrap_or_else(|_| json!({}));
        let receipt = (state == "stop" && reply["decision"] == "block").then_some(Receipt {
            session_id: session,
            callback_id: callback.callback_id,
        });
        return Ok((reply, receipt));
    }
    hooks::spool(&state_path()?, &callback)?;
    Ok((json!({}), None))
}

async fn confirm(socket: &Path, receipt: &Receipt) -> Result<()> {
    // Write the receipt before transport so a daemon restart can restore delivery.
    let pending = delivery::spool(&state_path()?, receipt)?;
    if matches!(
        tokio::time::timeout(
            TRANSPORT_DEADLINE,
            client::request(
                socket,
                hyper::Method::POST,
                "/v1/hooks/delivered",
                serde_json::to_vec(receipt)?,
            )
        )
        .await,
        Ok(Ok(_))
    ) {
        std::fs::remove_file(pending)?;
    }
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    env::var_os("WS_STATE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws/state")))
        .ok_or_else(|| "Set HOME or WS_STATE to locate the hook spool.".into())
}
