use crate::{Result, client};
use hyper::Method;
use std::path::Path;
use ws_core::registry::Workspace;

pub(super) struct Row {
    pub state: String,
    pub name: String,
    pub path: String,
    pub agent: String,
    pub window: String,
    pub pinned: bool,
    pub active: bool,
    pub unread: bool,
}

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let workspaces = workspaces(socket).await?;
    if json {
        println!("{}", serde_json::to_string(&workspaces)?);
        return Ok(());
    }
    for row in workspace_rows(workspaces) {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.state,
            row.name,
            row.path,
            row.agent,
            row.window,
            u8::from(row.pinned),
            u8::from(row.active),
            u8::from(row.unread)
        );
    }
    Ok(())
}

pub(super) async fn rows(socket: &Path) -> Result<Vec<Row>> {
    Ok(workspace_rows(workspaces(socket).await?))
}

pub(super) async fn workspaces(socket: &Path) -> Result<Vec<Workspace>> {
    let parent = super::caller(socket).await?;
    let path = parent.map_or_else(
        || "/v1/workspaces".into(),
        |id| format!("/v1/workspaces?parent_session_id={id}"),
    );
    let body = client::request(socket, Method::GET, &path, Vec::new()).await?;
    Ok(serde_json::from_slice(&body)?)
}

fn workspace_rows(workspaces: Vec<Workspace>) -> Vec<Row> {
    let live = ws_daemon::tmux::Tmux::from_env()
        .execute(&[
            "list-windows",
            "-a",
            "-F",
            "#{window_id}\t#{@ws_session}\t#{@ws_state}\t#{window_active}\t#{?@ws_unread,1,0}",
        ])
        .unwrap_or_default();
    let mut rows = Vec::new();
    for workspace in workspaces {
        let path = workspace.body["path"].as_str().unwrap_or("");
        let pinned = workspace.body["pinned"].as_bool().unwrap_or(false);
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
            rows.push(Row {
                state: fields[2].into(),
                name,
                path: path.into(),
                agent: window["agent"].as_str().unwrap_or("-").into(),
                window: fields[0].into(),
                pinned,
                active: fields[3] == "1",
                unread: fields[4] == "1",
            });
        }
        if !found {
            rows.push(Row {
                state: "stopped".into(),
                name: workspace.id,
                path: path.into(),
                agent: "-".into(),
                window: String::new(),
                pinned,
                active: false,
                unread: false,
            });
        }
    }
    rows
}
