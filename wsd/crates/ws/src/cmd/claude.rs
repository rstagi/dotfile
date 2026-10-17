use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::{
    env,
    path::Path,
    process::{Command, ExitCode},
};
use ws_daemon::agents::launch::Launch;

pub async fn run(socket: &Path, agent: &str, arguments: Vec<String>) -> Result<ExitCode> {
    let pane = env::var("TMUX_PANE")
        .ok()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| format!("{agent}: run it inside a workspace's tmux tab"))?;
    let target =
        super::add::current_workspace().map_err(|_| format!("{agent}: not inside a workspace"))?;
    let parent_session_id = super::caller(socket).await?;
    let response = client::request(socket, Method::POST, "/v1/agents/run", serde_json::to_vec(&json!({"target": target, "pane": pane, "agent": agent, "arguments": arguments, "parent_session_id": parent_session_id}))?).await?;
    let response: serde_json::Value = serde_json::from_slice(&response)?;
    execute(
        socket,
        response["session_id"]
            .as_str()
            .ok_or("The launch response has no session id.")?,
        true,
    )
    .await
}

pub async fn execute(socket: &Path, id: &str, user_tab: bool) -> Result<ExitCode> {
    let pane = env::var("TMUX_PANE").map_err(|_| "Run the agent inside a tmux pane.")?;
    let response = client::request(
        socket,
        Method::POST,
        "/v1/agents/launch",
        serde_json::to_vec(&json!({"id": id, "pane": pane}))?,
    )
    .await?;
    let launch: Launch = serde_json::from_slice(&response)?;
    let result = tokio::task::spawn_blocking(move || {
        let status = Command::new(&launch.prepared.program)
            .args(&launch.prepared.argv)
            .envs(&launch.prepared.env)
            .current_dir(&launch.directory)
            .status();
        for file in launch.prepared.files {
            if file
                .path
                .file_name()
                .is_some_and(|name| name == "claude-settings.json")
            {
                let _ = std::fs::remove_file(&file.path);
                if let Some(parent) = file.path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        }
        status
    })
    .await?;
    let code = result
        .as_ref()
        .map_or(1, |status| status.code().unwrap_or(1));
    client::request(
        socket,
        Method::POST,
        "/v1/agents/exit",
        serde_json::to_vec(&json!({"id": id, "pane": pane, "code": code, "user_tab": user_tab}))?,
    )
    .await?;
    result?;
    if !user_tab && code != 0 {
        eprintln!("agent exited ({code}) — enter to close");
        tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)
        })
        .await??;
    }
    Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
}
