use std::{env, error::Error, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

mod client;
mod cmd;
mod hook;
mod init;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(version, about = "Control local WS sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the local device identity.
    Init,
    /// Serve the local API on the Unix socket.
    Daemon,
    /// Show the daemon version and uptime.
    Ping,
    /// List the registered sessions.
    Sessions {
        #[arg(long)]
        json: bool,
        target: Vec<String>,
    },
    /// Send a hook event from the current tmux pane.
    Hook {
        #[command(subcommand)]
        event: HookCommand,
    },
}

#[derive(Subcommand)]
enum HookCommand {
    /// Record the end of an agent turn.
    Stop,
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
    match cli.command {
        Command::Init => init::run()?,
        Command::Daemon => {
            let socket = socket_path()?;
            ws_daemon::run(&socket).await.map_err(|error| {
                format!(
                    "Cannot serve the daemon socket at {}: {error}",
                    socket.display()
                )
            })?;
        }
        Command::Ping => {
            let response = client::ping(&socket_path()?).await?;
            println!(
                "wsd {} (uptime: {}s)",
                response.version, response.uptime_secs
            );
        }
        Command::Sessions { json, target } => {
            cmd::sessions::run(&socket_path()?, &target, json).await?
        }
        Command::Hook {
            event: HookCommand::Stop,
        } => hook::stop(&socket_path()?).await?,
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
