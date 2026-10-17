use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: Option<String>, agent: String, detach: bool) -> Result<()> {
    let target = match target {
        Some(target) => target,
        None => current_workspace()?,
    };
    let parent_session_id = super::caller(socket).await?;
    let body = client::request(
        socket,
        Method::POST,
        "/v1/workspaces/add",
        serde_json::to_vec(
            &json!({"target": target, "agent": agent, "parent_session_id": parent_session_id}),
        )?,
    )
    .await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    super::new::focus(
        response["window_id"]
            .as_str()
            .ok_or("The workspace response has no window id.")?,
        detach,
    )
}

fn current_workspace() -> Result<String> {
    let root = std::env::var_os("WS_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".ws/worktrees"))
        })
        .and_then(|root| root.canonicalize().ok());
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if output.status.success() {
        let top = String::from_utf8(output.stdout)?.trim_end().to_owned();
        if let Some(root) = root
            && Path::new(&top)
                .strip_prefix(root)
                .is_ok_and(|relative| relative.components().count() == 2)
        {
            return Ok(top);
        }
    }
    let target = std::env::var("TMUX_PANE")
        .ok()
        .filter(|pane| !pane.is_empty())
        .or_else(|| {
            std::env::var_os("TMUX")
                .filter(|tmux| !tmux.is_empty())
                .map(|_| ":".into())
        });
    if let Some(target) = target
        && let Some(path) = ws_daemon::tmux::Tmux::from_env()
            .window_option(&target, "@ws_path")?
            .filter(|path| !path.is_empty())
    {
        return Ok(path);
    }
    Err("add: not inside a workspace (pass <repo/branch>)".into())
}
