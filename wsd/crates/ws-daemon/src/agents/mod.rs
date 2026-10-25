//! Agent launch preparation shared by interactive and headless sessions.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
};
use ws_core::ids::Ulid;

pub mod launch;
pub mod slots;
mod telemetry;
pub(crate) use launch::router;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Mode {
    Interactive,
    Headless,
}

pub struct AgentSession {
    pub agent_program: Option<PathBuf>,
    pub state: PathBuf,
    pub parent_session_id: Option<Ulid>,
    pub project: String,
    pub id: String,
    pub home: PathBuf,
    pub grok_home: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    pub ws_home: PathBuf,
    pub ws_bin: PathBuf,
    pub spawned: bool,
    pub prompt: Option<String>,
    pub arguments: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Prepared {
    pub program: String,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub unset_env: Vec<String>,
    pub files: Vec<GeneratedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryMetadata>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TelemetryMetadata {
    pub agent_version: Option<String>,
    pub settings_version: String,
    pub version_covered: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

/// Builds launch arguments and files. This function does not write configuration files.
pub fn prepare(
    agent: &str,
    mode: Mode,
    session: &AgentSession,
    resume: Option<&str>,
) -> io::Result<Prepared> {
    let loop_context = crate::loops::guard::context(
        &session.state,
        session
            .id
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        session.parent_session_id,
    )?;
    let guarded = loop_context.is_some();
    let mut prepared = Prepared {
        program: agent.into(),
        argv: vec![],
        env: BTreeMap::from([("WS_SESSION_ID".into(), session.id.clone())]),
        unset_env: vec![
            "WS_SESSION".into(),
            "WS_LOOP_ID".into(),
            "WS_LOOP_GUARD_AGENT".into(),
        ],
        files: vec![],
        telemetry: None,
    };
    if let Some(context) = loop_context {
        prepared
            .env
            .insert("WS_LOOP_ID".into(), context.id.to_string());
        prepared
            .env
            .insert("WS_LOOP_GUARD_AGENT".into(), agent.into());
    }
    let preamble = if session.spawned {
        format!(
            "You are ws session #{}; run `ws done` when the user's task is complete. Use the ws-sessions skill to coordinate sessions.",
            session.id
        )
    } else {
        format!(
            "You are ws session #{}. Use the ws-sessions skill to coordinate sessions.",
            session.id
        )
    };
    let hook = format!(
        "{} hook",
        shell_words::quote(&session.ws_bin.to_string_lossy())
    );
    match agent {
        "claude" => {
            let path = session
                .ws_home
                .join("launches")
                .join(Ulid::new().to_string())
                .join("claude-settings.json");
            prepared.files.push(GeneratedFile {
                path: path.clone(),
                contents: serde_json::to_vec(&hooks("claude", &hook, guarded))?,
            });
            let mcp_path = path.with_file_name("claude-mcp.json");
            prepared.files.push(GeneratedFile {
                path: mcp_path.clone(),
                contents: serde_json::to_vec(&json!({"mcpServers": {"ws": {
                    "command": session.ws_bin,
                    "args": ["mcp"],
                    "env": {"WS_SESSION_ID": session.id}
                }}}))?,
            });
            prepared.argv.extend([
                "--mcp-config".into(),
                mcp_path.to_string_lossy().into_owned(),
                "--dangerously-skip-permissions".into(),
                "--settings".into(),
                path.to_string_lossy().into_owned(),
                "--append-system-prompt".into(),
                preamble,
            ]);
            if let Some(resume) = resume.filter(|s| !s.is_empty()) {
                prepared.argv.extend(["--resume".into(), resume.into()]);
            }
            // Claude consumes positional prompts after variadic flags as flag values.
            // Put the prompt before all options, including caller-supplied options.
            if let Some(prompt) = &session.prompt {
                prepared.argv.insert(0, prompt.clone());
            }
            if matches!(mode, Mode::Headless) {
                prepared.argv.insert(0, "-p".into());
            }
        }
        "codex" => {
            if matches!(mode, Mode::Headless) {
                prepared.argv.push("exec".into());
            }
            if let Some(resume) = resume.filter(|s| !s.is_empty()) {
                prepared.argv.extend(["resume".into(), resume.into()]);
            }
            prepared.argv.extend([
                "--dangerously-bypass-approvals-and-sandbox".into(),
                "--dangerously-bypass-hook-trust".into(),
                "-c".into(),
                format!(
                    "developer_instructions={}",
                    serde_json::to_string(&preamble)?
                ),
            ]);
            for setting in [
                format!(
                    "mcp_servers.ws.command={}",
                    serde_json::to_string(&session.ws_bin)?
                ),
                "mcp_servers.ws.args=[\"mcp\"]".into(),
                format!(
                    "mcp_servers.ws.env.WS_SESSION_ID={}",
                    serde_json::to_string(&session.id)?
                ),
                "mcp_servers.ws.env_vars=[\"WS_SOCK\"]".into(),
            ] {
                prepared.argv.extend(["-c".into(), setting]);
            }
            for (event, state) in [
                ("UserPromptSubmit", "working"),
                ("PreToolUse", "working"),
                ("PostToolUse", "working"),
                ("PermissionRequest", "waiting"),
                ("Stop", "stop"),
                ("Interrupt", "cancel"),
            ] {
                let command = serde_json::to_string(&format!("{hook} {state}"))?;
                let guard = if guarded && event == "PreToolUse" {
                    format!(
                        ",{{type=\"command\",command={}}}",
                        serde_json::to_string(&format!("{hook} guard"))?
                    )
                } else {
                    String::new()
                };
                prepared.argv.extend([
                    "-c".into(),
                    format!(
                        "hooks.{event}=[{{hooks=[{{type=\"command\",command={command}}}{guard}]}}]"
                    ),
                ]);
            }
        }
        "grok" => {
            prepared.files.push(GeneratedFile {
                path: session
                    .grok_home
                    .clone()
                    .unwrap_or_else(|| session.home.join(".grok"))
                    .join("hooks/ws.json"),
                contents: serde_json::to_vec(&hooks("grok", &hook, guarded))?,
            });
            prepared
                .argv
                .extend(["--always-approve".into(), "--rules".into(), preamble]);
            if matches!(mode, Mode::Headless)
                && let Some(prompt) = &session.prompt
            {
                let path = session
                    .ws_home
                    .join("launches")
                    .join(Ulid::new().to_string())
                    .join("prompt");
                prepared
                    .argv
                    .extend(["--prompt-file".into(), path.to_string_lossy().into_owned()]);
                prepared.files.push(GeneratedFile {
                    path,
                    contents: prompt.as_bytes().to_vec(),
                });
            }
            if let Some(resume) = resume.filter(|s| !s.is_empty()) {
                prepared.argv.extend(["--resume".into(), resume.into()]);
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "agent must be one of: claude, codex, grok",
            ));
        }
    }
    if agent == "claude" {
        crate::quota::claude::prepare(&mut prepared, session)?;
    }
    if agent == "codex" {
        prepared
            .files
            .push(crate::quota::binding::prepare_file(session, resume)?);
    }
    if agent == "grok" {
        prepared
            .files
            .push(crate::quota::grok::prepare_file(session)?);
    }
    telemetry::extend(&mut prepared, agent, session)?;
    prepared.argv.extend(session.arguments.iter().cloned());
    if agent != "claude"
        && !(agent == "grok" && matches!(mode, Mode::Headless))
        && let Some(prompt) = &session.prompt
    {
        prepared.argv.extend(["--".into(), prompt.clone()]);
    }
    Ok(prepared)
}

impl Prepared {
    /// Writes generated files with private permissions. Replaces each file atomically.
    pub fn write_files(&self) -> io::Result<()> {
        use std::io::Write;
        for file in &self.files {
            let parent = file
                .path
                .parent()
                .ok_or_else(|| io::Error::other("The generated file has no parent directory."))?;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
            let temporary = parent.join(format!(".ws-{}", Ulid::new()));
            let result = (|| {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                output.write_all(&file.contents)?;
                fs::rename(&temporary, &file.path)
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result?;
        }
        Ok(())
    }
}

fn hooks(agent: &str, command: &str, guarded: bool) -> Value {
    let run = |state: &str| json!([{ "hooks": [{"type": "command", "command": format!("{command} {state}"), "timeout": if matches!(state, "stop" | "exit") { crate::hooks::STOP_CLI_BUDGET.as_secs() + 10 } else { 5 }}] }]);
    let mut settings = json!({"hooks": {
        "UserPromptSubmit": run("working"), "PreToolUse": run("working"), "PostToolUse": run("working"),
        "Stop": run("stop"), "PermissionRequest": run("waiting")
    }});
    let hooks = settings["hooks"].as_object_mut().unwrap();
    if guarded {
        hooks["PreToolUse"].as_array_mut().unwrap().push(json!({
            "hooks": [{"type":"command", "command":format!("{command} guard"), "timeout":5}]
        }));
    }
    if agent == "claude" {
        hooks.insert("SessionEnd".into(), run("exit"));
        // Claude runs hooks only after its trust dialog. SessionStart marks input readiness.
        hooks.insert("SessionStart".into(), run("ready"));
        hooks.insert("PostToolUseFailure".into(), run("working"));
        hooks.insert("SubagentStart".into(), run("subagent-start"));
        hooks.insert("SubagentStop".into(), run("subagent-stop"));
    } else {
        hooks.insert("StopCancelled".into(), run("cancel"));
    }
    hooks.insert("Notification".into(), json!([{
        "matcher": if agent == "claude" { "permission_prompt|elicitation_dialog" } else { "permission_prompt" },
        "hooks": [{"type": "command", "command": format!("{command} waiting"), "timeout": 5}]
    }]));
    settings
}
