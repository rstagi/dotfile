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

/// Hooks always succeed. A failed transport leaves its callback in the spool.
pub async fn run(socket: &Path, state: &str) {
    let _ = deliver(socket, state).await;
}

async fn deliver(socket: &Path, state: &str) -> Result<()> {
    let observed_at_us = hooks::receipt_time_us()?;
    let mut input = String::new();
    if !io::stdin().is_terminal() {
        io::stdin().read_to_string(&mut input)?;
    }
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({"raw":input}));
    let session = env::var("WS_SESSION_ID").or_else(|_| env::var("WS_SESSION"));
    let callback = Callback {
        callback_id: Ulid::new(),
        session_id: session
            .as_ref()
            .ok()
            .and_then(|id| id.parse().ok())
            .unwrap_or_default(),
        observed_at_us,
        state: state.into(),
        pane: env::var("TMUX_PANE").ok(),
        tmux_socket: env::var("WS_TMUX_SOCKET").ok(),
        tmux_path: env::var("TMUX")
            .ok()
            .and_then(|value| value.split(',').next().map(str::to_owned)),
        payload,
    };
    if session.is_err() {
        // Keep the anonymous S5 diagnostic path for callers without a tracked session.
        if state == "stop"
            && let Some(pane) = &callback.pane
        {
            let window = hooks::tmux(
                &callback,
                &["display-message", "-p", "-t", pane, "#{window_id}"],
            )
            .await?;
            let _ =
                tokio::time::timeout(Duration::from_millis(40), client::hook_stop(socket, window))
                    .await;
        }
        return Ok(());
    }
    let state_path = env::var_os("WS_STATE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws/state")))
        .ok_or("Set HOME or WS_STATE to locate the hook spool.")?;
    let path = hooks::spool(&state_path, &callback)?;
    if let Ok(Ok(reply)) = tokio::time::timeout(
        Duration::from_millis(40),
        client::request(
            socket,
            hyper::Method::POST,
            "/v1/hooks/ingest",
            serde_json::to_vec(&callback)?,
        ),
    )
    .await
    {
        std::fs::remove_file(path).or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })?;
        if state == "stop" {
            println!(
                "{}",
                serde_json::from_slice::<Value>(&reply).unwrap_or_else(|_| json!({}))
            );
        }
    }
    Ok(())
}
