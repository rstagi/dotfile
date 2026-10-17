use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Subcommand, ValueEnum};
use hyper::Method;
use tokio::signal::unix::{Signal, SignalKind, signal};
use ws_daemon::supervisor::CreateHeadless;

use crate::{Result, client};

#[derive(Subcommand)]
pub enum Command {
    /// Create a daemon-owned process.
    Create {
        #[arg(long, required = true)]
        headless: bool,
        #[command(flatten)]
        arguments: HeadlessArgs,
    },
    /// Run a daemon-owned process and wait for its exit.
    Run {
        #[arg(long)]
        headless: bool,
        #[command(flatten)]
        arguments: HeadlessArgs,
    },
    /// Read the session output.
    Tail {
        id: String,
        #[arg(long)]
        follow: bool,
        #[arg(long, value_enum, default_value = "stdout")]
        stream: Stream,
    },
}

#[derive(Args)]
pub struct HeadlessArgs {
    #[arg(long)]
    parent: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env", value_name = "K=V")]
    environment: Vec<String>,
    #[arg(long)]
    stdin: Option<PathBuf>,
    #[arg(long)]
    stdout: Option<PathBuf>,
    #[arg(long)]
    stderr: Option<PathBuf>,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

struct Signals {
    terminate: Signal,
    interrupt: Signal,
    hangup: Signal,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Stream {
    Stdout,
    Stderr,
}

pub async fn run(socket: &Path, command: Command) -> Result<ExitCode> {
    match command {
        Command::Create {
            headless,
            arguments,
        } => {
            let session = create(socket, arguments, headless).await?;
            println!("{}", session.id);
        }
        Command::Run { arguments, .. } => {
            // Register before creation so a pending signal cannot orphan a new session.
            let mut signals = Signals {
                terminate: signal(SignalKind::terminate())?,
                interrupt: signal(SignalKind::interrupt())?,
                hangup: signal(SignalKind::hangup())?,
            };
            let session = create(socket, arguments, true).await?;
            return wait(socket, session.id, &mut signals).await;
        }
        Command::Tail { id, follow, stream } => tail(socket, &id, follow, stream).await?,
    }
    Ok(ExitCode::SUCCESS)
}

async fn create(
    socket: &Path,
    arguments: HeadlessArgs,
    headless: bool,
) -> Result<ws_core::registry::Session> {
    let directory = std::env::current_dir()?;
    let absolute = |path: PathBuf| directory.join(path);
    let parent_session_id = match arguments.parent {
        Some(parent) => Some(client::session(socket, &parent).await?.id),
        None => None,
    };
    let mut env = BTreeMap::new();
    for assignment in arguments.environment {
        let (key, value) = assignment
            .split_once('=')
            .ok_or("Use K=V for each --env argument.")?;
        env.insert(key.to_owned(), value.to_owned());
    }
    let request = CreateHeadless {
        headless,
        command: arguments.command,
        cwd: arguments.cwd.map(absolute).unwrap_or(directory.clone()),
        parent_session_id,
        env,
        stdin: arguments.stdin.map(absolute),
        stdout: arguments.stdout.map(absolute),
        stderr: arguments.stderr.map(absolute),
    };
    let bytes = client::request(
        socket,
        Method::POST,
        "/v1/sessions",
        serde_json::to_vec(&request)?,
    )
    .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn wait(
    socket: &Path,
    id: ws_core::ids::SessionId,
    signals: &mut Signals,
) -> Result<ExitCode> {
    let id = id.to_string();
    loop {
        let session = client::session(socket, &id).await?;
        if ["finished", "failed"].contains(&session.state.as_str()) {
            return Ok(ExitCode::from(session.exit_code.unwrap_or(1) as u8));
        }
        tokio::select! {
            _ = signals.terminate.recv() => {},
            _ = signals.interrupt.recv() => {},
            _ = signals.hangup.recv() => {},
            _ = tokio::time::sleep(Duration::from_millis(50)) => continue,
        }
        client::request(
            socket,
            Method::POST,
            &format!("/v1/sessions/{id}/stop"),
            b"{}".to_vec(),
        )
        .await?;
    }
}

async fn tail(socket: &Path, id: &str, follow: bool, stream: Stream) -> Result<()> {
    let session = client::session(socket, id).await?;
    let field = match stream {
        Stream::Stdout => "stdout",
        Stream::Stderr => "stderr",
    };
    let path = session.body[field]
        .as_str()
        .ok_or("The session has no output file for this stream.")?;
    let mut file = File::open(PathBuf::from(path))?;
    let mut output = io::stdout();
    loop {
        io::copy(&mut file, &mut output)?;
        output.flush()?;
        if !follow {
            break;
        }
        let session = client::session(socket, &session.id.to_string()).await?;
        if ["finished", "failed"].contains(&session.state.as_str()) {
            io::copy(&mut file, &mut output)?;
            output.flush()?;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
