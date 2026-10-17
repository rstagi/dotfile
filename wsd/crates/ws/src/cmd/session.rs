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
    /// List registered sessions.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a dependency between two sessions.
    DependsOn {
        target: String,
        dependency: String,
        #[arg(long)]
        json: bool,
    },
    /// Block a session on another session.
    BlockOn {
        target: String,
        dependency: String,
        #[arg(long)]
        json: bool,
    },
    /// Stop a registered session.
    Stop {
        target: String,
        #[arg(long)]
        json: bool,
    },
    /// Read a registered session.
    Get {
        target: String,
        #[arg(long)]
        json: bool,
    },
    /// Create a daemon-owned process.
    Create {
        #[arg(long)]
        headless: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        workspace_id: Option<String>,
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
        #[arg(long, conflicts_with = "follow")]
        json: bool,
        #[arg(long, requires = "json")]
        max_bytes: Option<u32>,
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
    #[arg(last = true)]
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
        Command::List { .. } => {
            let value = crate::mcp::Control::from_env(socket)?
                .call("session_list", serde_json::json!({}))
                .await?;
            println!("{value}");
        }
        Command::DependsOn {
            target, dependency, ..
        } => {
            super::mcp::print(
                socket,
                "session_depends_on",
                serde_json::json!({"target":target,"dependency":dependency}),
            )
            .await?;
        }
        Command::BlockOn {
            target, dependency, ..
        } => {
            super::mcp::print(
                socket,
                "session_block_on",
                serde_json::json!({"target":target,"dependency":dependency}),
            )
            .await?;
        }
        Command::Stop { target, .. } => {
            super::mcp::print(socket, "session_stop", serde_json::json!({"target":target})).await?;
        }
        Command::Get { target, .. } => {
            super::mcp::print(socket, "session_get", serde_json::json!({"target":target})).await?;
        }
        Command::Create {
            headless,
            arguments,
            json,
            name,
            workspace_id,
        } => {
            if headless {
                if name.is_some() || workspace_id.is_some() {
                    return Err("Headless creation does not accept name or workspace_id.".into());
                }
                let session = create(socket, arguments, headless).await?;
                if json {
                    println!("{session}");
                } else {
                    println!(
                        "{}",
                        session["id"]
                            .as_str()
                            .ok_or("The daemon did not return a session id.")?
                    );
                }
            } else {
                if !arguments.command.is_empty()
                    || arguments.parent.is_some()
                    || arguments.cwd.is_some()
                    || !arguments.environment.is_empty()
                    || arguments.stdin.is_some()
                    || arguments.stdout.is_some()
                    || arguments.stderr.is_some()
                {
                    return Err("Use --headless to set process options.".into());
                }
                let value = crate::mcp::Control::from_env(socket)?
                    .call(
                        "session_create",
                        serde_json::json!({"name":name, "workspace_id":workspace_id}),
                    )
                    .await?;
                if json {
                    println!("{value}");
                } else {
                    println!(
                        "{}",
                        value["id"]
                            .as_str()
                            .ok_or("The daemon did not return a session id.")?
                    );
                }
            }
        }
        Command::Run { arguments, .. } => {
            // Register before creation so a pending signal cannot orphan a new session.
            let mut signals = Signals {
                terminate: signal(SignalKind::terminate())?,
                interrupt: signal(SignalKind::interrupt())?,
                hangup: signal(SignalKind::hangup())?,
            };
            let session = create(socket, arguments, true).await?;
            return wait(
                socket,
                serde_json::from_value(session["id"].clone())?,
                &mut signals,
            )
            .await;
        }
        Command::Tail {
            id,
            follow,
            stream,
            json,
            max_bytes,
        } => {
            if json {
                let stream = match stream {
                    Stream::Stdout => "stdout",
                    Stream::Stderr => "stderr",
                };
                super::mcp::print(
                    socket,
                    "session_tail",
                    serde_json::json!({"target":id,"stream":stream,"max_bytes":max_bytes}),
                )
                .await?;
            } else {
                tail(socket, &id, follow, stream).await?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn create(
    socket: &Path,
    arguments: HeadlessArgs,
    headless: bool,
) -> Result<serde_json::Value> {
    let directory = std::env::current_dir()?;
    let absolute = |path: PathBuf| directory.join(path);
    let parent_session_id = match arguments.parent {
        Some(parent) => Some(client::session(socket, &parent).await?.id),
        None => std::env::var("WS_SESSION_ID")
            .ok()
            .map(|id| id.parse())
            .transpose()?,
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
    let exited = process_exit(socket, &id);
    tokio::pin!(exited);
    loop {
        tokio::select! {
            event = &mut exited => {
                let code = event?["body"]["code"].as_i64().unwrap_or(1);
                return Ok(ExitCode::from(code as u8));
            }
            _ = signals.terminate.recv() => {},
            _ = signals.interrupt.recv() => {},
            _ = signals.hangup.recv() => {},
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

/// Waits for the process exit. `ws done` can finish the session earlier.
async fn process_exit(socket: &Path, id: &str) -> Result<serde_json::Value> {
    let condition = format!("event={}", ws_daemon::supervisor::PROCESS_EXITED);
    let bytes = client::request(
        socket,
        Method::POST,
        &format!("/v1/sessions/{id}/wait"),
        serde_json::to_vec(&serde_json::json!({"for": condition}))?,
    )
    .await?;
    let mut result: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(result["event"].take())
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
    let id = session.id.to_string();
    let exited = process_exit(socket, &id);
    tokio::pin!(exited);
    loop {
        io::copy(&mut file, &mut output)?;
        output.flush()?;
        if !follow {
            return Ok(());
        }
        tokio::select! {
            event = &mut exited => {
                event?;
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    io::copy(&mut file, &mut output)?;
    output.flush()?;
    Ok(())
}
