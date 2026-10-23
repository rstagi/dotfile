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

const DEFAULT_TRANSPORT_DEADLINE: Duration = Duration::from_millis(250);
/// Test seam: sandboxes under load set a long hang guard instead of the production deadline.
const TRANSPORT_DEADLINE_ENV: &str = "WS_HOOK_TRANSPORT_DEADLINE_MS";

/// Hooks always succeed. A failed transport leaves its callback in the spool.
pub async fn run(socket: &Path, state: &str) {
    let started = tokio::time::Instant::now();
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
        ("quota", _) => "quota",
        (_, "SessionEnd" | "session_end") => "exit",
        ("idle", "Stop" | "stop") => "stop",
        ("idle", "StopCancelled" | "stop_cancelled" | "Interrupt") => "cancel",
        _ => state,
    };
    let deadline =
        started + transport_deadline(state, env::var(TRANSPORT_DEADLINE_ENV).ok().as_deref());
    let (mut reply, mut receipt) = deliver(socket, state, payload, observed_at_us, deadline)
        .await
        .unwrap_or_else(|_| (json!({}), None));
    if state == "stop" {
        let _backend = ws_daemon::import::backend::home().ok().and_then(|home| {
            ws_daemon::import::backend::delivery_guard(&home)
                .ok()
                .flatten()
        });
        if _backend.is_none() {
            reply = json!({});
            receipt = None;
        }
        let written = {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_ok()
        };
        if let Some(receipt) = receipt.filter(|_| written) {
            let _ = confirm(socket, &receipt, deadline).await;
        }
    }
}

async fn deliver(
    socket: &Path,
    state: &str,
    payload: Value,
    observed_at_us: u64,
    deadline: tokio::time::Instant,
) -> Result<(Value, Option<Receipt>)> {
    let session = env::var("WS_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(|| env::var("WS_SESSION").ok().filter(|id| !id.is_empty()));
    let Some(session) = session else {
        return Ok((json!({}), None));
    };
    let Some(session) = resolve(socket, &session, deadline).await else {
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
        "quota",
        "exit",
        "ready",
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
    if let Ok(reply) = client::hook_request(
        socket,
        "/v1/hooks/ingest",
        serde_json::to_vec(&callback)?,
        deadline.saturating_duration_since(tokio::time::Instant::now()),
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

/// Legacy zsh agents export a numeric `WS_SESSION`. Only the daemon maps it to
/// the imported ULID, so an unreachable daemon drops that callback.
/// The production deadline keeps a hook from stalling the agent. A stop or exit hook waits
/// longer for the locked daemon ingest. A sandbox may set its own deadline.
fn transport_deadline(state: &str, value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(match state {
            "stop" | "exit" => hooks::STOP_CLI_BUDGET,
            _ => DEFAULT_TRANSPORT_DEADLINE,
        })
}

async fn resolve(socket: &Path, identity: &str, deadline: tokio::time::Instant) -> Option<Ulid> {
    if let Ok(id) = identity.parse() {
        return Some(id);
    }
    let target = if identity.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("#{identity}")
    } else {
        identity.to_owned()
    };
    tokio::time::timeout_at(deadline, client::session(socket, &target))
        .await
        .ok()?
        .ok()
        .map(|session| session.id)
}

async fn confirm(socket: &Path, receipt: &Receipt, deadline: tokio::time::Instant) -> Result<()> {
    // Write the receipt before transport so a daemon restart can restore delivery.
    let pending = delivery::spool(&state_path()?, receipt)?;
    if client::hook_request(
        socket,
        "/v1/hooks/delivered",
        serde_json::to_vec(receipt)?,
        deadline.saturating_duration_since(tokio::time::Instant::now()),
    )
    .await
    .is_ok()
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transport_deadline_defaults_to_250ms_and_accepts_a_sandbox_override() {
        for state in [
            "working",
            "idle",
            "cancel",
            "waiting",
            "quota",
            "subagent-stop",
        ] {
            assert_eq!(transport_deadline(state, None), Duration::from_millis(250));
            assert_eq!(
                transport_deadline(state, Some("")),
                Duration::from_millis(250)
            );
            assert_eq!(
                transport_deadline(state, Some("abc")),
                Duration::from_millis(250)
            );
            assert_eq!(
                transport_deadline(state, Some("0")),
                Duration::from_millis(250)
            );
            assert_eq!(
                transport_deadline(state, Some(" 30000 ")),
                Duration::from_secs(30)
            );
        }
    }

    #[test]
    fn stop_and_exit_hooks_have_a_20s_total_budget_and_accept_a_sandbox_override() {
        for state in ["stop", "exit"] {
            assert_eq!(transport_deadline(state, None), Duration::from_secs(20));
            assert_eq!(transport_deadline(state, Some("")), Duration::from_secs(20));
            assert_eq!(
                transport_deadline(state, Some("0")),
                Duration::from_secs(20)
            );
            assert_eq!(
                transport_deadline(state, Some(" 30000 ")),
                Duration::from_secs(30)
            );
            assert_eq!(
                transport_deadline(state, Some("100")),
                Duration::from_millis(100)
            );
        }
    }
}
