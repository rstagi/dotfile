use std::{env, error::Error, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

mod client;
mod cmd;
mod init;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Parser)]
#[command(version, about = "Control local WS sessions")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run Claude in the current workspace tab.
    Claude {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
    /// Run Codex in the current workspace tab.
    Codex {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
    /// Run Grok in the current workspace tab.
    Grok {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
    #[command(name = "_agent", hide = true)]
    Agent { id: String },
    /// Merge the current workspace PR.
    Merge,
    #[command(name = "_merge_popup", hide = true)]
    MergePopup,
    /// Focus or resume a workspace or session.
    Open {
        #[arg(default_value = "")]
        target: String,
        #[arg(long)]
        detach: bool,
    },
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
    /// Create or read a headless session.
    Session {
        #[command(subcommand)]
        command: cmd::session::Command,
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
    #[command(name = "_build-notifier", hide = true)]
    BuildNotifier,
    #[command(name = "_dismiss", hide = true)]
    Dismiss { target: Option<String> },
    /// Send a hook event from the current tmux pane.
    Hook { state: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Hooks only perform one socket request. Avoid creating a worker pool for each callback.
    let mut builder = if matches!(cli.command, Some(Command::Hook { .. })) {
        tokio::runtime::Builder::new_current_thread()
    } else {
        tokio::runtime::Builder::new_multi_thread()
    };
    let result = match builder.enable_all().build() {
        Ok(runtime) => runtime.block_on(run(cli)),
        Err(error) => Err(error.into()),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("ws: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let Some(command) = cli.command else {
        return cmd::attach::run(&socket_path()?)
            .await
            .map(|()| ExitCode::SUCCESS);
    };
    match command {
        Command::Claude { arguments } => {
            return cmd::claude::run(&socket_path()?, arguments).await;
        }
        Command::Codex { arguments } => return cmd::codex::run(&socket_path()?, arguments).await,
        Command::Grok { arguments } => return cmd::grok::run(&socket_path()?, arguments).await,
        Command::Agent { id } => return cmd::agent::execute(&socket_path()?, &id, false).await,
        Command::Merge => cmd::merge::run(&socket_path()?).await?,
        Command::MergePopup => return cmd::merge::popup(&socket_path()?).await,
        Command::Open { target, detach } => {
            cmd::open::run(&socket_path()?, &target, detach).await?
        }
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
        Command::Session { command } => return cmd::session::run(&socket_path()?, command).await,
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
        Command::BuildNotifier => cmd::build_notifier::run()?,
        Command::Dismiss { target } => {
            cmd::dismiss::run(&socket_path().unwrap_or_default(), target.as_deref()).await;
        }
        Command::Hook { state } => {
            cmd::hook::run(&socket_path().unwrap_or_default(), &state).await;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn socket_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WS_SOCK") {
        return Ok(path.into());
    }
    let home = env::var_os("HOME").ok_or("Set HOME or WS_SOCK to locate the daemon socket.")?;
    Ok(PathBuf::from(home).join(".ws/wsd.sock"))
}
