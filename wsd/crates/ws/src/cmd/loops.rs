use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::json;

use crate::{Result, mcp::Control};

#[derive(Subcommand)]
pub enum Command {
    /// Create the loop session of a loop directory. The session has no process.
    Create {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Start a prepared Loop orchestrator as a headless session.
    Start {
        loop_dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Pause a loop through its dashboard control file.
    Stop {
        loop_dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(socket: &Path, command: Command) -> Result<()> {
    let (tool, directory, json_output) = match command {
        Command::Create { dir, json } => ("loop_create", dir, json),
        Command::Start { loop_dir, json } => ("loop_start", loop_dir, json),
        Command::Stop { loop_dir, json } => ("loop_stop", loop_dir, json),
    };
    let response = Control::from_env(socket)?
        .call(tool, json!({"loop_dir":directory}))
        .await?;
    if json_output {
        println!("{response}");
    } else if tool != "loop_stop" {
        println!(
            "{}",
            response["id"]
                .as_str()
                .ok_or("The daemon did not return a session id.")?
        );
    } else {
        println!(
            "Paused {}",
            response["run_id"]
                .as_str()
                .ok_or("The daemon did not return a run id.")?
        );
    }
    Ok(())
}
