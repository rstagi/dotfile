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
    let target = super::open::normalize_target(target)?;
    let directory = ws_daemon::workspaces::directory_path(&target);
    let scratch = match &directory {
        Some(directory) => ws_daemon::workspaces::is_scratch_directory(directory)?,
        None => false,
    };
    let mut confirmed = false;
    let parent_session_id = super::caller(socket).await?;
    if interactive || scratch {
        let name = match &directory {
            Some(directory) => directory.display().to_string(),
            None => ws_daemon::workspaces::workspace_path(&target)?.0,
        };
        let mut tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        if scratch {
            write!(tty, "Delete {name} and stop all its sessions? [y/N] ")?;
        } else if directory.is_some() {
            write!(tty, "Stop all sessions in {name}? [y/N] ")?;
        } else {
            write!(
                tty,
                "remove {name} and all its sessions? [y/N/b=also delete branch] "
            )?;
        }
        tty.flush()?;
        let mut answer = String::new();
        std::io::BufReader::new(tty).read_line(&mut answer)?;
        match answer.trim() {
            "y" | "Y" => {}
            "b" | "B" if directory.is_none() => delete_branch = true,
            _ => return Ok(()),
        }
        confirmed = true;
    }
    let body = client::request(socket, Method::POST, "/v1/workspaces/remove", serde_json::to_vec(&json!({
        "target": target, "force": force, "delete_branch": delete_branch, "force_delete_branch": force_delete_branch, "parent_session_id": parent_session_id, "confirmed": confirmed
    }))?).await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    if let Some(stderr) = response["git_stderr"].as_str() {
        eprint!("{stderr}");
    }
    if let Some(warning) = response["warning"].as_str() {
        eprintln!("ws: {warning}");
    }
    Ok(())
}
