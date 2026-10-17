//! Daemon-owned processes and durable exit records.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use ws_core::{
    ids::{SessionId, Ulid},
    registry::Session,
    state::SessionState,
};

use crate::{
    agents::{self, AgentSession, Mode},
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateHeadless {
    pub headless: bool,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub parent_session_id: Option<SessionId>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub stdin: Option<PathBuf>,
    pub stdout: Option<PathBuf>,
    pub stderr: Option<PathBuf>,
}

pub(crate) async fn create(
    store: Store,
    request: CreateHeadless,
) -> std::result::Result<Session, ApiError> {
    launch(store, request, serde_json::Value::Null).await
}

pub(crate) async fn launch(
    store: Store,
    request: CreateHeadless,
    metadata: serde_json::Value,
) -> std::result::Result<Session, ApiError> {
    let monitor_store = store.clone();
    api::call(store, move |sessions| {
        let (session, mut child) = sessions.spawn_headless(request, metadata)?;
        let id = session.id;
        let parent = session.body["parent_session_id"]
            .as_str()
            .and_then(|id| id.parse().ok());
        if session.body.get("phase").is_some()
            && let Some(parent) = parent
        {
            sessions.append(
                parent,
                "phase.spawned",
                json!({
                    "parent_session_id": parent, "session_id": id,
                    "phase": session.body["phase"], "repo": session.body["repo"],
                    "base": session.body["base"],
                }),
            )?;
        }
        tokio::spawn(async move {
            let status = child.wait().await;
            let result = api::call(monitor_store, move |sessions| {
                let status = status?;
                let code = status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
                sessions.append(
                    id,
                    "exit",
                    json!({"code": code, "signal": status.signal(), "parent_session_id": parent}),
                )?;
                sessions.transition(
                    &id.to_string(),
                    if code == 0 {
                        SessionState::Finished
                    } else {
                        SessionState::Failed
                    },
                    parent,
                )?;
                Ok(())
            })
            .await;
            if let Err(error) = result {
                eprintln!("Cannot record the exit for session {id}: {}", error.0);
            }
        });
        Ok(session)
    })
    .await
}

impl Sessions {
    fn spawn_headless(
        &mut self,
        request: CreateHeadless,
        metadata: serde_json::Value,
    ) -> Result<(Session, tokio::process::Child)> {
        if !request.headless || request.command.first().is_none_or(|s| s.is_empty()) {
            return Err(Error::InvalidRequest(
                "Specify a command for the headless session.",
            ));
        }
        for (key, value) in &request.env {
            if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
                return Err(Error::InvalidRequest(
                    "Each environment key must be nonempty and contain no equals sign or NUL.",
                ));
            }
        }
        let cwd = request.cwd.canonicalize()?;
        if !cwd.is_dir() {
            return Err(Error::InvalidRequest(
                "The working directory is not a directory.",
            ));
        }
        let id = Ulid::new();
        self.record_call(
            request.parent_session_id,
            "session.create",
            Some(&id.to_string()),
        )?;
        let directory = self
            .state
            .canonicalize()?
            .join("sessions")
            .join(id.to_string());
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let mut command = headless_command(&request, id)?;
        let stdout = request
            .stdout
            .unwrap_or_else(|| directory.join("stdout.log"));
        let stderr = request
            .stderr
            .unwrap_or_else(|| directory.join("stderr.log"));
        let input = request.stdin.map(File::open).transpose()?;
        let output = output_file(&stdout)?;
        let errors = output_file(&stderr)?;
        if same_file(&output, &errors)?
            || input
                .as_ref()
                .map(|input| same_file(input, &output))
                .transpose()?
                .unwrap_or(false)
            || input
                .as_ref()
                .map(|input| same_file(input, &errors))
                .transpose()?
                .unwrap_or(false)
        {
            return Err(Error::InvalidRequest(
                "Use separate files for stdin, stdout and stderr.",
            ));
        }
        output.set_len(0)?;
        errors.set_len(0)?;
        let mut body = json!({
            "alias": self.registry.next_alias()?, "kind": "headless",
            "command": request.command, "cwd": cwd, "stdout": stdout, "stderr": stderr,
            "parent_session_id": request.parent_session_id,
        });
        if let Some(metadata) = metadata.as_object() {
            body.as_object_mut().unwrap().extend(metadata.clone());
        }
        self.append(id, "session.created", body)?;
        command
            .current_dir(cwd)
            .envs(&request.env)
            .env("WS_SESSION_ID", id.to_string())
            .env("WS_SESSION", id.to_string())
            .stdin(input.map(Stdio::from).unwrap_or_else(Stdio::null))
            .stdout(output)
            .stderr(errors)
            .process_group(0)
            .kill_on_drop(true);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.transition(
                    &id.to_string(),
                    SessionState::Failed,
                    request.parent_session_id,
                )?;
                return Err(error.into());
            }
        };
        let pid = child.id().expect("A newly spawned child has a pid.");
        self.append(
            id,
            "session.running",
            json!({
                "parent_session_id": request.parent_session_id,
                "process": {"pid": pid, "pgid": pid},
            }),
        )?;
        Ok((self.get_record(&id.to_string())?, child))
    }
}

fn headless_command(request: &CreateHeadless, id: SessionId) -> io::Result<Command> {
    let mut command = Command::new(&request.command[0]);
    let agent = Path::new(&request.command[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if ["claude", "codex", "grok"].contains(&agent) {
        let environment = |key: &str| {
            request
                .env
                .get(key)
                .map(std::ffi::OsString::from)
                .or_else(|| std::env::var_os(key))
        };
        let home =
            PathBuf::from(environment("HOME").ok_or_else(|| {
                io::Error::other("Set HOME to locate agent configuration files.")
            })?);
        let mut arguments = request.command[1..].to_vec();
        if arguments.first().is_some_and(|argument| match agent {
            "claude" => ["-p", "--print"].contains(&argument.as_str()),
            "codex" => argument == "exec",
            _ => false,
        }) {
            arguments.remove(0);
        }
        let context = AgentSession {
            id: id.to_string(),
            ws_home: environment("WS_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".ws")),
            grok_home: environment("GROK_HOME").map(PathBuf::from),
            home,
            ws_bin: std::env::current_exe()?,
            spawned: false,
            prompt: None,
            arguments,
        };
        let prepared = agents::prepare(agent, Mode::Headless, &context, None)?;
        prepared.write_files()?;
        command.args(prepared.argv).envs(prepared.env);
    } else {
        command.args(&request.command[1..]);
    }
    Ok(command)
}

fn output_file(path: &PathBuf) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)
}

fn same_file(left: &File, right: &File) -> io::Result<bool> {
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}
