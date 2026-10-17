use crate::{Result, client, cmd};
use hyper::Method;
use serde_json::{Value, json};
use std::path::Path;

pub async fn run(socket: &Path, arguments: Vec<String>) -> Result<()> {
    let caller = cmd::caller(socket).await?;
    if arguments.first().map(String::as_str) == Some("--rm") {
        if arguments.len() != 2 {
            return Err("on --rm: expected <trigger-id>".into());
        }
        let id: ws_core::ids::Ulid = arguments[1]
            .parse()
            .map_err(|_| "Use a valid trigger id.")?;
        client::request(
            socket,
            Method::POST,
            &format!("/v1/triggers/{id}/remove"),
            serde_json::to_vec(&json!({"parent_session_id":caller}))?,
        )
        .await?;
        return Ok(());
    }
    if arguments.len() < 4 {
        return Err(
            "on: expected <ids> <done|exit> --send-to <id> or --spawn <agent> [prompt]".into(),
        );
    }
    let sources: Vec<_> = arguments[0].split(',').collect();
    if sources.iter().any(|source| source.is_empty()) {
        return Err("Specify a session id between each comma.".into());
    }
    let mut resolved = Vec::new();
    for source in sources {
        resolved.push(resolve(socket, source, caller).await?);
    }
    let mut request = json!({"sources":resolved, "event":arguments[1], "parent_session_id":caller});
    match arguments[2].as_str() {
        "--send-to" => {
            request["target"] = json!(resolve(socket, &arguments[3], caller).await?);
            match &arguments[4..] {
                [] => {}
                [template] => request["template"] = json!(template),
                [flag, template] if flag == "--template" => request["template"] = json!(template),
                _ => return Err("on: --send-to expects a target and an optional template.".into()),
            }
        }
        "--spawn" => {
            let rest = &arguments[3..];
            match rest {
                [agent] if is_agent(agent) => request["spawn"] = json!(agent),
                [agent, prompt] if is_agent(agent) => {
                    request["spawn"] = json!(agent);
                    request["template"] = json!(prompt);
                }
                [agent, flag, prompt] if is_agent(agent) && flag == "--template" => {
                    request["spawn"] = json!(agent);
                    request["template"] = json!(prompt);
                }
                // Keep the zsh prompt-first form during the backend transition.
                [prompt] => {
                    request["spawn"] = json!("claude");
                    request["template"] = json!(prompt);
                }
                [prompt, flag, agent] if flag == "--agent" => {
                    request["spawn"] = json!(agent);
                    request["template"] = json!(prompt);
                }
                _ => return Err("on: --spawn expects an agent and an optional prompt.".into()),
            }
            if caller.is_none() {
                let path = std::env::current_dir()?;
                let (name, _) = ws_daemon::workspaces::workspace_path(
                    path.to_str().ok_or("The workspace path is not UTF-8.")?,
                )?;
                request["workspace"] = json!(name);
            }
        }
        _ => return Err("on: expected --send-to or --spawn".into()),
    }
    let response = client::request(
        socket,
        Method::POST,
        "/v1/triggers",
        serde_json::to_vec(&request)?,
    )
    .await?;
    let trigger: Value = serde_json::from_slice(&response)?;
    println!(
        "{}",
        trigger["id"]
            .as_str()
            .ok_or("The trigger reply has no id.")?
    );
    Ok(())
}

async fn resolve(
    socket: &Path,
    input: &str,
    caller: Option<ws_core::ids::SessionId>,
) -> Result<String> {
    if input == "me" {
        return caller
            .map(|id| id.to_string())
            .ok_or_else(|| "The current terminal has no session id.".into());
    }
    Ok(client::session(socket, input).await?.id.to_string())
}

fn is_agent(agent: &str) -> bool {
    ["claude", "codex", "grok"].contains(&agent)
}
