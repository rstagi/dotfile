use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::{
    io::{BufRead, Write},
    path::Path,
};

pub async fn run(
    socket: &Path,
    target: &str,
    force: bool,
    mut delete_branch: bool,
    force_delete_branch: bool,
    interactive: bool,
) -> Result<()> {
    let parent_session_id = super::caller(socket).await?;
    if interactive {
        let body = client::request(socket, Method::GET, "/v1/workspaces", Vec::new()).await?;
        let workspaces: Vec<ws_core::registry::Workspace> = serde_json::from_slice(&body)?;
        let workspace = workspaces
            .iter()
            .find(|workspace| {
                workspace.id == target
                    || workspace.body["path"].as_str().is_some_and(|path| {
                        Path::new(target).is_absolute()
                            && Path::new(path).canonicalize().ok()
                                == Path::new(target).canonicalize().ok()
                    })
            })
            .ok_or_else(|| format!("no such workspace: {target}"))?;
        let mut tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        write!(
            tty,
            "remove {} and all its sessions? [y/N/b=also delete branch] ",
            workspace.id
        )?;
        tty.flush()?;
        let mut answer = String::new();
        std::io::BufReader::new(tty).read_line(&mut answer)?;
        match answer.trim() {
            "y" | "Y" => {}
            "b" | "B" => delete_branch = true,
            _ => return Ok(()),
        }
    }
    let body = client::request(socket, Method::POST, "/v1/workspaces/remove", serde_json::to_vec(&json!({
        "target": target, "force": force, "delete_branch": delete_branch, "force_delete_branch": force_delete_branch, "parent_session_id": parent_session_id
    }))?).await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    if let Some(warning) = response["warning"].as_str() {
        eprintln!("ws: {warning}");
    }
    Ok(())
}
