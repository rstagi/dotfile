use std::path::Path;

use serde_json::json;
use ws_daemon::wait::Condition;

use crate::Result;

#[derive(clap::Args)]
pub struct Args {
    #[arg(required = true)]
    targets: Vec<String>,
    #[arg(long = "for")]
    condition: String,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long, value_parser = ["any", "all"])]
    mode: Option<String>,
    /// Repeat once per target, in target order. Omit this option to use zero for every target.
    #[arg(long)]
    after_seq: Vec<u64>,
    #[arg(long)]
    json: bool,
}

pub async fn run(socket: &Path, arguments: Args) -> Result<()> {
    Condition::parse(&arguments.condition)?;
    if !arguments.after_seq.is_empty() && arguments.after_seq.len() != arguments.targets.len() {
        return Err("Supply one --after-seq value per target, in target order.".into());
    }
    let request = if arguments.targets.len() == 1 && arguments.mode.is_none() {
        json!({"target":arguments.targets[0],"condition":arguments.condition,
            "timeout":arguments.timeout,"after_seq":arguments.after_seq.first().copied()})
    } else {
        let targets: Vec<_> = arguments.targets.iter().enumerate().map(|(index,target)|
            json!({"target":target,"after_seq":arguments.after_seq.get(index).copied().unwrap_or(0)})).collect();
        json!({"targets":targets,"mode":arguments.mode.unwrap_or_else(|| "all".into()),
            "condition":arguments.condition,"timeout":arguments.timeout})
    };
    let value = crate::mcp::Control::for_caller(socket, super::caller(socket).await?)
        .call("wait", request)
        .await?;
    if arguments.json {
        println!("{value}");
    }
    Ok(())
}
