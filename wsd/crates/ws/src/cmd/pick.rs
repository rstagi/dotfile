use std::process::{Command, ExitCode};

use crate::Result;

const SCRIPT: &str = include_str!("picker.zsh");

pub fn run() -> Result<ExitCode> {
    configure_bindings()?;
    let status = shell("pick")?.status()?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

pub fn configure_bindings() -> Result<()> {
    if !shell("bindings")?.status()?.success() {
        return Err("Cannot configure the tmux bindings.".into());
    }
    Ok(())
}

pub fn focus(window: &str) -> Result<()> {
    super::new::focus(window, false)
}

fn shell(action: &str) -> Result<Command> {
    let mut command = Command::new("zsh");
    command
        .args(["-fc", SCRIPT, "ws-picker", action])
        .env("WS_BIN", std::env::current_exe()?)
        .env("WS_PICKER_RUST", "1");
    Ok(command)
}
