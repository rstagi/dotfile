use std::{env, path::PathBuf};

use crate::Result;

pub fn run(dry_run: bool) -> Result<()> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let source = env::var_os("WS_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".ws")))
        .ok_or("Set HOME or WS_HOME to locate the zsh state.")?;
    let state = env::var_os("WS_STATE")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".ws/state")))
        .ok_or("Set HOME or WS_STATE to locate the Rust state.")?;
    let plan = ws_daemon::import::from_zsh(&source, &state, dry_run)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}
