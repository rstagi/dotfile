use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(
    socket: &Path,
    repo: String,
    branch: String,
    agent: String,
    detach: bool,
) -> Result<()> {
    let body = client::request(
        socket,
        Method::POST,
        "/v1/workspaces",
        serde_json::to_vec(&json!({"repo": repo, "branch": branch, "agent": agent}))?,
    )
    .await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    focus(
        response["window_id"]
            .as_str()
            .ok_or("The workspace response has no window id.")?,
        detach,
    )
}

pub(super) fn focus(window: &str, detach: bool) -> Result<()> {
    if !detach {
        let tmux = ws_daemon::tmux::Tmux::from_env();
        if std::env::var_os("TMUX").is_some() {
            tmux.execute(&["switch-client", "-t", window])?;
        } else {
            let mut command = std::process::Command::new("tmux");
            if let Some(socket) =
                std::env::var_os("WS_TMUX_SOCKET").filter(|socket| !socket.is_empty())
            {
                command.arg("-L").arg(socket);
            }
            if !command
                .args(["attach-session", "-t", window])
                .status()?
                .success()
            {
                return Err("Cannot attach to the workspace.".into());
            }
        }
    }
    Ok(())
}
