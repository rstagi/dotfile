use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: Option<String>, agent: String, detach: bool) -> Result<()> {
    let target = match target {
        Some(target) => {
            let path = super::new::expand_home(&target)?;
            if path.is_dir() {
                path.canonicalize()?
                    .into_os_string()
                    .into_string()
                    .map_err(|_| "The directory path is not UTF-8.")?
            } else {
                target
            }
        }
        None => current_place()?,
    };
    let parent_session_id = super::spawner(socket).await?;
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
            .ok_or("The session response has no window id.")?,
        detach,
    )
}

pub(super) fn current_workspace() -> Result<String> {
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
    tmux_path()?.ok_or_else(|| "add: not inside a workspace (pass <repo/branch>)".into())
}

fn current_place() -> Result<String> {
    match tmux_path()? {
        Some(path) => Ok(path),
        None => std::env::current_dir()?
            .into_os_string()
            .into_string()
            .map_err(|_| "The current directory path is not UTF-8.".into()),
    }
}

fn tmux_path() -> Result<Option<String>> {
    let target = std::env::var("TMUX_PANE")
        .ok()
        .filter(|pane| !pane.is_empty())
        .or_else(|| {
            std::env::var_os("TMUX")
                .filter(|tmux| !tmux.is_empty())
                .map(|_| ":".into())
        });
    match target {
        Some(target) => Ok(ws_daemon::tmux::Tmux::from_env()
            .window_option(&target, "@ws_path")?
            .filter(|path| !path.is_empty())),
        None => Ok(None),
    }
}
