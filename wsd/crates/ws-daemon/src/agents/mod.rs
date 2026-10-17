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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Mode {
    Interactive,
    Headless,
}

pub struct AgentSession {
    pub id: String,
    pub home: PathBuf,
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
    pub files: Vec<GeneratedFile>,
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
    let mut prepared = Prepared {
        program: agent.into(),
        argv: vec![],
        env: BTreeMap::from([
            ("WS_SESSION".into(), session.id.clone()),
            ("WS_SESSION_ID".into(), session.id.clone()),
        ]),
        files: vec![],
    };
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
                contents: serde_json::to_vec(&hooks("claude", &hook))?,
            });
            prepared.argv.extend([
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
            if let Some(resume) = resume.filter(|s| !s.is_empty()) {
                prepared.argv.extend(["resume".into(), resume.into()]);
            } else if matches!(mode, Mode::Headless) {
                prepared.argv.push("exec".into());
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
            for (event, state) in [
                ("UserPromptSubmit", "working"),
                ("PreToolUse", "working"),
                ("PostToolUse", "working"),
                ("PermissionRequest", "waiting"),
                ("Stop", "stop"),
                ("Interrupt", "cancel"),
            ] {
                let command = serde_json::to_string(&format!("{hook} {state}"))?;
                prepared.argv.extend([
                    "-c".into(),
                    format!("hooks.{event}=[{{hooks=[{{type=\"command\",command={command}}}]}}]"),
                ]);
            }
        }
        "grok" => {
            prepared.files.push(GeneratedFile {
                path: session.home.join(".grok/hooks/ws.json"),
                contents: serde_json::to_vec(&hooks("grok", &hook))?,
            });
            prepared
                .argv
                .extend(["--always-approve".into(), "--rules".into(), preamble]);
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
    prepared.argv.extend(session.arguments.iter().cloned());
    if agent != "claude"
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

fn hooks(agent: &str, command: &str) -> Value {
    let run = |state: &str| json!([{ "hooks": [{"type": "command", "command": format!("{command} {state}"), "timeout": 5}] }]);
    let mut settings = json!({"hooks": {
        "UserPromptSubmit": run("working"), "PreToolUse": run("working"), "PostToolUse": run("working"),
        "Stop": run("stop"), "PermissionRequest": run("waiting")
    }});
    let hooks = settings["hooks"].as_object_mut().unwrap();
    if agent == "claude" {
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
