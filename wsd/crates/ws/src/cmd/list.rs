use crate::{Result, client};
use hyper::Method;
use std::path::Path;
use ws_core::registry::Workspace;

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let body = client::request(socket, Method::GET, "/v1/workspaces", Vec::new()).await?;
    let workspaces: Vec<Workspace> = serde_json::from_slice(&body)?;
    if json {
        println!("{}", serde_json::to_string(&workspaces)?);
        return Ok(());
    }
    let tmux = ws_daemon::tmux::Tmux::from_env();
    let live = tmux
        .execute(&[
            "list-windows",
            "-a",
            "-F",
            "#{window_id}\t#{@ws_session}\t#{@ws_state}\t#{window_active}\t#{?@ws_unread,1,0}",
        ])
        .unwrap_or_default();
    for workspace in workspaces {
        let path = workspace.body["path"].as_str().unwrap_or("");
        let pinned = if workspace.body["pinned"].as_bool().unwrap_or(false) {
            "1"
        } else {
            "0"
        };
        let mut found = false;
        for window in workspace.body["windows"].as_array().into_iter().flatten() {
            let Some(fields) = live
                .lines()
                .map(|line| line.split('\t').collect::<Vec<_>>())
                .find(|fields| {
                    fields.len() == 5
                        && Some(fields[0]) == window["window_id"].as_str()
                        && Some(fields[1]) == window["session_id"].as_str()
                })
            else {
                continue;
            };
            found = true;
            let slot = window["slot"].as_u64().unwrap_or(1);
            let name = if slot > 1 {
                format!("{}#{slot}", workspace.id)
            } else {
                workspace.id.clone()
            };
            println!(
                "{}\t{name}\t{path}\t{}\t{}\t{pinned}\t{}\t{}",
                fields[2],
                window["agent"].as_str().unwrap_or("-"),
                fields[0],
                fields[3],
                fields[4]
            );
        }
        if !found {
            println!("stopped\t{}\t{path}\t-\t\t{pinned}\t0\t0", workspace.id);
        }
    }
    Ok(())
}
