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
    /// Remove a workspace and all its tabs.
    Rm {
        #[arg(default_value = "")]
        target: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        delete_branch: bool,
        #[arg(long)]
        force_delete_branch: bool,
        #[arg(long)]
        interactive: bool,
    },
    /// Toggle the workspace pin.
    Pin {
        #[arg(default_value = "")]
        target: String,
    },
    /// Initialize the local device identity.
    Init,
    /// Serve the local API on the Unix socket.
    Daemon,
    /// Show the daemon version and uptime.
    Ping,
    /// Create a workspace and its first session.
    New {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long, default_value = "claude")]
        agent: String,
        #[arg(long)]
        detach: bool,
    },
    /// Add a session to a workspace.
    Add {
        target: Option<String>,
        #[arg(long, default_value = "claude")]
        agent: String,
        #[arg(long)]
        detach: bool,
    },
    /// List the registered workspaces.
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show the current session identity.
    Whoami {
        #[arg(long)]
        json: bool,
    },
    /// List the registered sessions.
    Sessions {
        #[arg(long)]
        json: bool,
        target: Vec<String>,
    },
    /// Set the current session name.
    Name {
        #[arg(long)]
        json: bool,
        label: Vec<String>,
    },
    /// Show one session and its output path.
    Show {
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
        Command::Rm {
            target,
            force,
            delete_branch,
            force_delete_branch,
            interactive,
        } => {
            cmd::rm::run(
                &socket_path()?,
                &target,
                force,
                delete_branch,
                force_delete_branch,
                interactive,
            )
            .await?
        }
        Command::Pin { target } => cmd::pin::run(&socket_path()?, &target).await?,
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
        Command::New {
            repo,
            branch,
            agent,
            detach,
        } => cmd::new::run(&socket_path()?, repo, branch, agent, detach).await?,
        Command::Add {
            target,
            agent,
            detach,
        } => cmd::add::run(&socket_path()?, target, agent, detach).await?,
        Command::List { json } => cmd::list::run(&socket_path()?, json).await?,
        Command::Whoami { json } => cmd::whoami::run(&socket_path()?, json).await?,
        Command::Sessions { json, target } => {
            cmd::sessions::run(&socket_path()?, &target, json).await?
        }
        Command::Name { json, label } => cmd::name::run(&socket_path()?, &label, json).await?,
        Command::Show { json, target } => cmd::show::run(&socket_path()?, &target, json).await?,
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
