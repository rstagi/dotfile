//! Discovers listed branch worktrees without adding arbitrary directories.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::append_once;
use crate::sessions::Sessions;

#[derive(Debug, Deserialize, Serialize)]
pub struct LegacyWorkspace {
    pub id: String,
    pub path: PathBuf,
    pub branch: String,
    pub pinned: bool,
}

pub(super) fn snapshot(source: &Path, root: &Path) -> io::Result<Vec<LegacyWorkspace>> {
    let pins = match fs::read_to_string(source.join("pins")) {
        Ok(pins) => pins,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut workspaces = Vec::new();
    for repo in fs::read_dir(&root)? {
        let repo = repo?;
        if !repo.file_type()?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(repo.path())? {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let path = path.canonicalize()?;
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            if relative.components().count() != 2 {
                continue;
            }
            let Some(branch) = listed_branch(&path)? else {
                continue;
            };
            let id = relative
                .to_str()
                .ok_or_else(|| io::Error::other("The workspace name is not UTF-8."))?
                .to_owned();
            workspaces.push(LegacyWorkspace {
                pinned: pins.lines().any(|pin| pin == id),
                id,
                path,
                branch,
            });
        }
    }
    workspaces.sort_by(|left, right| left.id.cmp(&right.id));
    workspaces.dedup_by(|left, right| left.id == right.id);
    Ok(workspaces)
}

pub(super) fn import(sessions: &mut Sessions, workspaces: &[LegacyWorkspace]) -> io::Result<()> {
    if workspaces.is_empty() {
        return Ok(());
    }
    let owner = sessions.message_session(None).map_err(io::Error::other)?.id;
    for workspace in workspaces {
        append_once(
            sessions,
            owner,
            &format!("workspace:{}", workspace.id),
            "workspace.created",
            json!({
                "workspace_id":workspace.id, "path":workspace.path, "branch":workspace.branch,
                "pinned":workspace.pinned, "windows":[], "tmux_session":workspace.id.replace(['.', ':'], "_")
            }),
        )?;
    }
    Ok(())
}

fn listed_branch(path: &Path) -> io::Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&output.stdout).map_err(io::Error::other)?;
    for record in text.split("\0\0") {
        let mut fields = record.split('\0');
        let Some(directory) = fields
            .next()
            .and_then(|field| field.strip_prefix("worktree "))
        else {
            continue;
        };
        if Path::new(directory).canonicalize().ok().as_deref() != Some(path) {
            continue;
        }
        return Ok(
            fields.find_map(|field| field.strip_prefix("branch refs/heads/").map(str::to_owned))
        );
    }
    Ok(None)
}
