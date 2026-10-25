use std::{
    collections::{BTreeSet, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use hyper::Method;
use serde_json::Value;
use ws_core::{
    ids::SessionId,
    registry::{Session, Workspace},
};

use crate::{Result, client};

pub async fn run(socket: &Path, once: bool, worktrees: bool, graph: bool) -> Result<ExitCode> {
    let current = if super::whoami::identity().is_some() {
        Some(super::whoami::current(socket).await?)
    } else {
        None
    };
    let (directory, run_id) = match current {
        Some(current) => resolve(socket, &current).await?,
        None => (env::current_dir()?, None),
    };
    let script = ws_daemon::loops::script_path("LOOP_TOP_MJS", "loop-top.mjs")?;
    let mut command = tokio::process::Command::new("node");
    command.arg(script).current_dir(directory);
    if let Some(run_id) = run_id {
        command.arg(run_id);
    }
    for (enabled, flag) in [
        (once, "--once"),
        (worktrees, "--worktrees"),
        (graph, "--graph"),
    ] {
        if enabled {
            command.arg(flag);
        }
    }
    let status = command.status().await?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

async fn resolve(socket: &Path, current: &Session) -> Result<(PathBuf, Option<String>)> {
    let sessions = client::sessions(socket).await?;
    if let Some(found) = wsd_loop(&sessions, current)? {
        return Ok(found);
    }
    let body = client::request(
        socket,
        Method::GET,
        &format!("/v1/workspaces?parent_session_id={}", current.id),
        Vec::new(),
    )
    .await?;
    let workspaces: Vec<Workspace> = serde_json::from_slice(&body)?;
    let directory = session_directory(current, &workspaces).unwrap_or(env::current_dir()?);
    let mut candidates = BTreeSet::new();
    add_ancestors(&directory, &mut candidates);
    for session in &sessions {
        if let Some(directory) = session_directory(session, &workspaces) {
            add_ancestors(&directory, &mut candidates);
        }
        if let Some(path) = session.body["loop_dir"].as_str() {
            candidates.insert(PathBuf::from(path));
        }
    }
    for workspace in &workspaces {
        if let Some(path) = workspace.body["path"].as_str() {
            add_ancestors(Path::new(path), &mut candidates);
        }
    }
    // Store records locate coordinator directories. The live state owns the session link.
    let store = env::var_os("LOOP_STORE_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".loop/loops")));
    if let Some(store) = store
        && let Ok(entries) = fs::read_dir(store)
    {
        for entry in entries.flatten() {
            if let Some(record) = read_json(&entry.path())
                && let Some(path) = record["loopDir"].as_str()
            {
                candidates.insert(PathBuf::from(path));
            }
        }
    }
    let states: Vec<_> = candidates
        .into_iter()
        .filter_map(|path| read_json(&path.join("state.json")).map(|state| (path, state)))
        .collect();
    let mut ancestor = Some(current.id);
    let mut visited = HashSet::new();
    while let Some(id) = ancestor {
        if !visited.insert(id) {
            return Err("The session ancestry contains a cycle.".into());
        }
        for (path, state) in &states {
            if state["wsSession"].as_str() == Some(id.to_string().as_str())
                && let Some(run_id) = state["runId"].as_str().filter(|id| !id.is_empty())
            {
                return Ok((
                    path.parent()
                        .ok_or("The loop directory has no parent.")?
                        .into(),
                    Some(run_id.into()),
                ));
            }
        }
        ancestor = sessions
            .iter()
            .find(|session| session.id == id)
            .and_then(|session| session.body["parent_session_id"].as_str())
            .map(str::parse::<SessionId>)
            .transpose()?;
    }
    Ok((directory, None))
}

/// Finds the loop session in the ancestry of the current session. wsd knows it without a scan.
fn wsd_loop(sessions: &[Session], current: &Session) -> Result<Option<(PathBuf, Option<String>)>> {
    let mut ancestor = Some(current.id);
    let mut visited = HashSet::new();
    while let Some(id) = ancestor {
        if !visited.insert(id) {
            return Err("The session ancestry contains a cycle.".into());
        }
        let Some(session) = sessions.iter().find(|session| session.id == id) else {
            break;
        };
        if session.body["kind"] == "loop"
            && let Some(loop_dir) = session.body["loop_dir"].as_str().map(Path::new)
        {
            let directory = loop_dir
                .parent()
                .ok_or("The loop directory has no parent.")?;
            return Ok(Some((directory.into(), Some(id.to_string()))));
        }
        ancestor = session.body["parent_session_id"]
            .as_str()
            .map(str::parse::<SessionId>)
            .transpose()?;
    }
    Ok(None)
}

fn session_directory(session: &Session, workspaces: &[Workspace]) -> Option<PathBuf> {
    session.body["cwd"].as_str().map(PathBuf::from).or_else(|| {
        workspaces
            .iter()
            .find(|workspace| Some(&workspace.id) == session.workspace_id.as_ref())
            .and_then(|workspace| workspace.body["path"].as_str())
            .map(PathBuf::from)
    })
}

fn add_ancestors(directory: &Path, candidates: &mut BTreeSet<PathBuf>) {
    for ancestor in directory.ancestors() {
        candidates.insert(ancestor.join(".loop"));
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
