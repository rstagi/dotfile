use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Subcommand;
use serde_json::json;

use crate::{Result, mcp::Control};

#[derive(Subcommand)]
pub enum Command {
    /// Start a local phase through the session supervisor.
    Spawn {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        base: String,
        #[arg(long)]
        max_runtime: u64,
        #[arg(long = "caps-required")]
        caps_required: Vec<String>,
        #[arg(long)]
        json: bool,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

pub async fn run(socket: &Path, command: Command) -> Result<ExitCode> {
    match command {
        Command::Spawn {
            repo,
            base,
            max_runtime,
            caps_required,
            json,
            command,
        } => {
            let response = Control::from_env(socket)?.call("phase_spawn", json!({"repo":repo,"base":base,"max_runtime":max_runtime,"caps_required":caps_required,"phase":{"command":command}})).await?;
            if json {
                println!("{response}");
            } else {
                println!(
                    "{}",
                    response["id"]
                        .as_str()
                        .ok_or("The daemon did not return a session id.")?
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
