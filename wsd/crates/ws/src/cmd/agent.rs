use crate::{Result, client};
use clap::{Subcommand, ValueEnum};
use hyper::Method;
use serde_json::json;
use std::{
    env,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode},
};
use ws_daemon::agents::{self, AgentSession, Mode, launch::Launch};

#[derive(Subcommand)]
pub enum Command {
    /// Write hook files and print launch arguments and environment variables.
    Prepare {
        #[arg(long, value_parser = ["claude", "codex", "grok"])]
        agent: String,
        #[arg(long, value_parser = ["headless"])]
        mode: String,
        #[arg(long)]
        session: ws_core::ids::SessionId,
        #[arg(long)]
        resume: Option<String>,
        #[arg(long, value_enum, default_value = "json")]
        format: Format,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Json,
    Sh,
}

pub fn prepare(command: Command) -> Result<()> {
    let Command::Prepare {
        agent,
        session,
        resume,
        format,
        ..
    } = command;
    let directory = env::current_dir()?;
    let home =
        directory.join(env::var_os("HOME").ok_or("Set HOME to locate agent configuration files.")?);
    let ws_home = env::var_os("WS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".ws"));
    let context = AgentSession {
        id: session.to_string(),
        home,
        grok_home: env::var_os("GROK_HOME").map(|path| directory.join(path)),
        ws_home: directory.join(ws_home),
        ws_bin: env::current_exe()?,
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    let prepared = agents::prepare(&agent, Mode::Headless, &context, resume.as_deref())?;
    prepared.write_files()?;
    match format {
        Format::Json => println!("{}", serde_json::to_string(&prepared)?),
        Format::Sh => {
            println!("WS_AGENT_PROGRAM={}", shell_words::quote(&prepared.program));
            let paths = shell_words::join(
                prepared
                    .files
                    .iter()
                    .map(|file| file.path.to_string_lossy()),
            );
            println!("WS_AGENT_FILES={}", shell_words::quote(&paths));
            for (key, value) in &prepared.env {
                println!("export {key}={}", shell_words::quote(value));
            }
            println!("set -- {}", shell_words::join(&prepared.argv));
        }
    }
    Ok(())
}

pub async fn run(socket: &Path, agent: &str, arguments: Vec<String>) -> Result<ExitCode> {
    let pane = env::var("TMUX_PANE")
        .ok()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| format!("{agent}: run it inside a workspace's tmux tab"))?;
    let target =
        super::add::current_workspace().map_err(|_| format!("{agent}: not inside a workspace"))?;
    let parent_session_id = super::caller(socket).await?;
    let response = client::request(socket, Method::POST, "/v1/agents/run", serde_json::to_vec(&json!({"target": target, "pane": pane, "agent": agent, "arguments": arguments, "parent_session_id": parent_session_id}))?).await?;
    let response: serde_json::Value = serde_json::from_slice(&response)?;
    execute(
        socket,
        response["session_id"]
            .as_str()
            .ok_or("The launch response has no session id.")?,
        true,
    )
    .await
}

pub async fn execute(socket: &Path, id: &str, user_tab: bool) -> Result<ExitCode> {
    let pane = env::var("TMUX_PANE").map_err(|_| "Run the agent inside a tmux pane.")?;
    let response = client::request(
        socket,
        Method::POST,
        "/v1/agents/launch",
        serde_json::to_vec(&json!({"id": id, "pane": pane}))?,
    )
    .await?;
    let launch: Launch = serde_json::from_slice(&response)?;
    let shell = env::var("WS_AGENT_SHELL").unwrap_or_else(|_| "zsh -ic".into());
    let shell = shell_words::split(&shell)?;
    let shell_program = shell
        .first()
        .ok_or("WS_AGENT_SHELL must contain a shell command.")?
        .clone();
    let result = tokio::task::spawn_blocking(move || {
        let command = shell_words::join(
            std::iter::once(launch.prepared.program.as_str())
                .chain(launch.prepared.argv.iter().map(String::as_str)),
        );
        let status = ProcessCommand::new(shell_program)
            .args(&shell[1..])
            .arg(command)
            .envs(&launch.prepared.env)
            .current_dir(&launch.directory)
            .status();
        for file in launch.prepared.files {
            if file
                .path
                .file_name()
                .is_some_and(|name| name == "claude-settings.json" || name == "claude-mcp.json")
            {
                let _ = std::fs::remove_file(&file.path);
                if let Some(parent) = file.path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        }
        status
    })
    .await?;
    let code = result.as_ref().map_or(1, |status| {
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
    });
    client::request(
        socket,
        Method::POST,
        "/v1/agents/exit",
        serde_json::to_vec(&json!({"id": id, "pane": pane, "code": code, "user_tab": user_tab}))?,
    )
    .await?;
    result?;
    if !user_tab && code != 0 {
        eprintln!("agent exited ({code}) — enter to close");
        tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)
        })
        .await??;
    }
    Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
}
