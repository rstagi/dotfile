use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use clap::{Subcommand, ValueEnum};
use hyper::Method;
use ws_daemon::supervisor::CreateHeadless;

use crate::{Result, client};

#[derive(Subcommand)]
pub enum Command {
    /// Create a daemon-owned process.
    Create {
        #[arg(long, required = true)]
        headless: bool,
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

#[derive(Clone, Copy, ValueEnum)]
pub enum Stream {
    Stdout,
    Stderr,
}

pub async fn run(socket: &Path, command: Command) -> Result<()> {
    match command {
        Command::Create {
            headless,
            parent,
            cwd,
            environment,
            stdin,
            stdout,
            stderr,
            command,
        } => {
            let directory = std::env::current_dir()?;
            let absolute = |path: PathBuf| directory.join(path);
            let parent_session_id = match parent {
                Some(parent) => Some(client::session(socket, &parent).await?.id),
                None => None,
            };
            let mut env = BTreeMap::new();
            for assignment in environment {
                let (key, value) = assignment
                    .split_once('=')
                    .ok_or("Use K=V for each --env argument.")?;
                env.insert(key.to_owned(), value.to_owned());
            }
            let request = CreateHeadless {
                headless,
                command,
                cwd: cwd.map(absolute).unwrap_or(directory.clone()),
                parent_session_id,
                env,
                stdin: stdin.map(absolute),
                stdout: stdout.map(absolute),
                stderr: stderr.map(absolute),
            };
            let bytes = client::request(
                socket,
                Method::POST,
                "/v1/sessions",
                serde_json::to_vec(&request)?,
            )
            .await?;
            let session: ws_core::registry::Session = serde_json::from_slice(&bytes)?;
            println!("{}", session.id);
        }
        Command::Tail { id, follow, stream } => tail(socket, &id, follow, stream).await?,
    }
    Ok(())
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
