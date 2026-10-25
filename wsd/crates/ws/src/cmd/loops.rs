use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::json;

use crate::{Result, mcp::Control};

#[derive(Subcommand)]
pub enum Command {
    /// Read the state of one loop from its session events.
    Get {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// List the loop states from their session events.
    List {
        #[arg(long)]
        json: bool,
    },
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
    let (tool, arguments, json_output) = match command {
        Command::Get { id, json } => ("loop_get", json!({"id":id}), json),
        Command::List { json } => ("loop_list", json!({}), json),
        Command::Create { dir, json } => ("loop_create", json!({"loop_dir":dir}), json),
        Command::Start { loop_dir, json } => ("loop_start", json!({"loop_dir":loop_dir}), json),
        Command::Stop { loop_dir, json } => ("loop_stop", json!({"loop_dir":loop_dir}), json),
    };
    let response = Control::from_env(socket)?.call(tool, arguments).await?;
    if json_output {
        println!("{response}");
    } else if matches!(tool, "loop_get" | "loop_list") {
        println!("{response:#}");
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
