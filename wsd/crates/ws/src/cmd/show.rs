use std::{
    env,
    path::{Path, PathBuf},
};

use crate::{Result, client};

pub async fn run(socket: &Path, targets: &[String], json: bool) -> Result<()> {
    let [target] = targets else {
        return Err("show: expected <id|name>".into());
    };
    let session = if target == "me" {
        super::whoami::current(socket).await?
    } else {
        client::session(socket, target).await?
    };
    let path = state_path()?
        .join("sessions")
        .join(session.id.to_string())
        .join("output.md");
    let output = if path.is_file() {
        path.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    if json {
        let mut value = super::session_json(&session)?;
        value["output"] = output.into();
        println!("{}", value);
    } else {
        println!(
            "id: #{}\nname: {}\nworkspace: {}\nagent: {}\nstatus: {}\noutput: {}",
            session.alias,
            session.name.as_deref().unwrap_or(""),
            session.workspace_id.as_deref().unwrap_or(""),
            super::agent(&session),
            super::status(&session),
            output
        );
    }
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WS_STATE") {
        return Ok(path.into());
    }
    let home = env::var_os("HOME").ok_or("Set HOME or WS_STATE to locate the session state.")?;
    Ok(PathBuf::from(home).join(".ws/state"))
}
