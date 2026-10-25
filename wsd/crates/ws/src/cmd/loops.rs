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
    /// Check the events of one loop against the Loop rules.
    Check {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Acknowledge a Loop violation with a reason.
    Ack {
        id: String,
        #[arg(long)]
        reason: String,
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
    /// Pause a loop or one phase.
    Pause {
        id: String,
        phase: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Resume a paused loop or phase.
    Resume {
        id: String,
        phase: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Record guidance for one phase.
    Note {
        id: String,
        phase: u64,
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Set the model for the next phase attempt.
    Model {
        id: String,
        phase: u64,
        chain: String,
        #[arg(long)]
        json: bool,
    },
    /// Accept the open accept gate.
    Accept {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Reject the open accept gate.
    Reject {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Change the tier of a review phase before it starts.
    Tier {
        id: String,
        phase: u64,
        #[arg(value_parser = ["standard", "max"])]
        tier: String,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5))]
        rounds: Option<u8>,
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
        Command::Check { id, json } => ("loop_check", json!({"id":id}), json),
        Command::Ack { id, reason, json } => ("loop_ack", json!({"id":id,"reason":reason}), json),
        Command::List { json } => ("loop_list", json!({}), json),
        Command::Create { dir, json } => ("loop_create", json!({"loop_dir":dir}), json),
        Command::Start { loop_dir, json } => ("loop_start", json!({"loop_dir":loop_dir}), json),
        Command::Pause { id, phase, json } => ("loop_pause", json!({"id":id,"phase":phase}), json),
        Command::Resume { id, phase, json } => {
            ("loop_resume", json!({"id":id,"phase":phase}), json)
        }
        Command::Note {
            id,
            phase,
            text,
            json,
        } => (
            "loop_note",
            json!({"id":id,"phase":phase,"text":text}),
            json,
        ),
        Command::Model {
            id,
            phase,
            chain,
            json,
        } => (
            "loop_model",
            json!({"id":id,"phase":phase,"model":chain}),
            json,
        ),
        Command::Accept { id, json } => ("loop_accept", json!({"id":id}), json),
        Command::Reject { id, json } => ("loop_reject", json!({"id":id}), json),
        Command::Tier {
            id,
            phase,
            tier,
            rounds,
            json,
        } => (
            "loop_tier",
            json!({"id":id,"phase":phase,"tier":tier,"rounds":rounds}),
            json,
        ),
        Command::Stop { loop_dir, json } => ("loop_stop", json!({"loop_dir":loop_dir}), json),
    };
    let response = Control::from_env(socket)?.call(tool, arguments).await?;
    if json_output {
        println!("{response}");
    } else if tool == "loop_check" {
        print_check(&response);
    } else if matches!(
        tool,
        "loop_get"
            | "loop_ack"
            | "loop_list"
            | "loop_pause"
            | "loop_resume"
            | "loop_note"
            | "loop_model"
            | "loop_accept"
            | "loop_reject"
            | "loop_tier"
    ) {
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

fn print_check(result: &serde_json::Value) {
    let violations = result["violations"].as_array().into_iter().flatten();
    for violation in violations {
        let seqs: Vec<String> = violation["seqs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|seq| seq.to_string())
            .collect();
        println!(
            "{} {} phase={} seq={} {}",
            violation["severity"].as_str().unwrap_or("?"),
            violation["rule"].as_str().unwrap_or("?"),
            violation["phase"]
                .as_u64()
                .map_or_else(|| "-".into(), |phase| phase.to_string()),
            seqs.join(","),
            violation["message"].as_str().unwrap_or_default()
        );
    }
    println!(
        "{}: {} hard, {} soft",
        if result["ok"] == true {
            "ok"
        } else {
            "violations"
        },
        result["hard"],
        result["soft"]
    );
}
