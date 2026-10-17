use crate::{Result, client};
use hyper::Method;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use ws_core::registry::Workspace;

pub async fn run(socket: &Path) -> Result<()> {
    let parent = super::caller(socket).await?;
    let path = parent.map_or_else(
        || "/v1/workspaces".into(),
        |id| format!("/v1/workspaces?parent_session_id={id}"),
    );
    let body = client::request(socket, Method::GET, &path, Vec::new()).await?;
    let mut workspaces: Vec<Workspace> = serde_json::from_slice(&body)?;
    if workspaces.is_empty() {
        return super::new::run(socket, None, None, "claude".into(), false).await;
    }
    workspaces.sort_by(|left, right| {
        let pinned = |workspace: &Workspace| !workspace.body["pinned"].as_bool().unwrap_or(false);
        (pinned(left), &left.id).cmp(&(pinned(right), &right.id))
    });
    let mut child = Command::new("fzf")
        .args([
            "--delimiter",
            "\t",
            "--with-nth",
            "1",
            "--no-sort",
            "--layout",
            "reverse",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "fzf not installed")?;
    let written = (|| -> std::io::Result<()> {
        let mut input = child.stdin.take().unwrap();
        for workspace in &workspaces {
            writeln!(
                input,
                "{}\t{}",
                workspace.id,
                workspace.body["path"].as_str().unwrap_or("")
            )?;
        }
        Ok(())
    })();
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Ok(());
    }
    written?;
    let selection = String::from_utf8(output.stdout)?;
    if let Some(row) = selection.lines().next().filter(|row| !row.is_empty()) {
        let target = row.split('\t').next().unwrap();
        super::open::run(socket, target, false).await?;
    }
    Ok(())
}
