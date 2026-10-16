use std::{env, error::Error, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use tokio::net::UnixStream;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(version, about = "Control local WS sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
        Command::Ping => {
            UnixStream::connect(&socket).await.map_err(|error| {
                format!(
                    "Cannot connect to the daemon at {}: {error}. Start it with `ws daemon`.",
                    socket.display()
                )
            })?;
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
