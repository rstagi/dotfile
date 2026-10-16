use std::{env, error::Error, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

mod client;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(version, about = "Control local WS sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the local API on the Unix socket.
    Daemon,
    /// Show the daemon version and uptime.
    Ping,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ws: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let socket = socket_path()?;
    match cli.command {
        Command::Daemon => ws_daemon::run(&socket).await.map_err(|error| {
            format!(
                "Cannot serve the daemon socket at {}: {error}",
                socket.display()
            )
        })?,
        Command::Ping => {
            let response = client::ping(&socket).await?;
            println!(
                "wsd {} (uptime: {}s)",
                response.version, response.uptime_secs
            );
        }
    }
    Ok(())
}

fn socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WS_SOCK") {
        return Ok(path.into());
    }
    let home = env::var_os("HOME").ok_or("Set HOME or WS_SOCK to locate the daemon socket.")?;
    Ok(PathBuf::from(home).join(".ws/wsd.sock"))
}
