use crate::{Result, client};
use hyper::Method;
use std::path::Path;
use ws_core::{
    ids::SessionId,
    registry::{Session, Workspace},
};

pub(super) struct Row {
    /// The session in this window. Stopped workspaces have none.
    pub session: Option<SessionId>,
    /// The session that created this row's session.
    pub parent: Option<SessionId>,
    /// The session name, or its `#N` alias.
    pub label: String,
    pub state: String,
    pub name: String,
    pub path: String,
    pub agent: String,
    pub window: String,
    pub pinned: bool,
    pub active: bool,
    pub unread: bool,
    pub scratch: bool,
}

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let workspaces = workspaces(socket).await?;
    if json {
        println!("{}", serde_json::to_string(&workspaces)?);
        return Ok(());
    }
    for row in workspace_rows(
        workspaces,
        &client::sessions(socket).await?,
        &live_windows(),
    ) {
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
    let workspaces = workspaces(socket).await?;
    let sessions = client::sessions(socket).await?;
    let live = live_windows();
    let mut rows = workspace_rows(workspaces, &sessions, &live);
    rows.extend(directory_rows(&sessions, &live));
    Ok(rows)
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

fn workspace_rows(workspaces: Vec<Workspace>, sessions: &[Session], live: &str) -> Vec<Row> {
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
                        && (Some(fields[1]) == window["session_id"].as_str()
                            || sessions.iter().any(|session| {
                                Some(session.id.to_string().as_str())
                                    == window["session_id"].as_str()
                                    && super::sessions::matches_identity(session, fields[1])
                            }))
                })
            else {
                continue;
            };
            found = true;
            let session = sessions.iter().find(|session| {
                Some(session.id.to_string().as_str()) == window["session_id"].as_str()
            });
            let slot = window["slot"].as_u64().unwrap_or(1);
            let name = if slot > 1 {
                format!("{}#{slot}", workspace.id)
            } else {
                workspace.id.clone()
            };
            rows.push(Row {
                session: session.map(|session| session.id),
                parent: session.and_then(super::parent),
                label: session.map(super::label).unwrap_or_default(),
                state: fields[2].into(),
                name,
                path: path.into(),
                agent: window["agent"].as_str().unwrap_or("-").into(),
                window: fields[0].into(),
                pinned,
                active: fields[3] == "1",
                unread: fields[4] == "1",
                scratch: false,
            });
        }
        if !found {
            rows.push(Row {
                session: None,
                parent: None,
                label: String::new(),
                state: "stopped".into(),
                name: workspace.id,
                path: path.into(),
                agent: "-".into(),
                window: String::new(),
                pinned,
                active: false,
                unread: false,
                scratch: false,
            });
        }
    }
    rows
}

fn directory_rows(sessions: &[Session], live: &str) -> Vec<Row> {
    sessions
        .iter()
        .filter(|session| {
            session.workspace_id.is_none()
                && session.body["kind"] != "headless"
                && !["finished", "failed"].contains(&session.state.as_str())
        })
        .filter_map(|session| {
            let path = session.cwd.as_ref()?;
            let fields = live
                .lines()
                .map(|line| line.split('\t').collect::<Vec<_>>())
                .find(|fields| {
                    fields.len() == 5 && super::sessions::matches_identity(session, fields[1])
                });
            let (state, window, active, unread) = fields
                .map_or(("stopped", "", false, false), |fields| {
                    (fields[2], fields[0], fields[3] == "1", fields[4] == "1")
                });
            Some(Row {
                session: Some(session.id),
                parent: super::parent(session),
                label: super::label(session),
                state: state.into(),
                name: format!("#{}", session.alias),
                path: path.clone(),
                agent: super::agent(session).into(),
                window: window.into(),
                pinned: false,
                active,
                unread,
                scratch: true,
            })
        })
        .collect()
}

fn live_windows() -> String {
    ws_daemon::tmux::Tmux::from_env()
        .execute(&[
            "list-windows",
            "-a",
            "-F",
            "#{window_id}\t#{@ws_session}\t#{@ws_state}\t#{window_active}\t#{?@ws_unread,1,0}",
        ])
        .unwrap_or_default()
}
