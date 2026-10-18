//! Private launch metadata. This file contains no credential.

use crate::agents::{AgentSession, GeneratedFile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Binding {
    pub runner: String,
    pub account_id: Option<String>,
    pub codex_home: PathBuf,
    pub program: PathBuf,
    pub resume_id: Option<String>,
}

/// Reuses the first binding when the same WS session resumes.
pub(crate) fn prepare_file(
    session: &AgentSession,
    resume: Option<&str>,
) -> io::Result<GeneratedFile> {
    let path = session
        .state
        .join("sessions")
        .join(&session.id)
        .join("quota-binding.json");
    let contents = match fs::read(&path) {
        Ok(contents) => {
            serde_json::from_slice::<Binding>(&contents).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The launch quota binding is invalid.",
                )
            })?;
            contents
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let codex_home = session
                .codex_home
                .clone()
                .unwrap_or_else(|| session.home.join(".codex"));
            let account_id = fs::read(codex_home.join("auth.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|auth| {
                    auth["tokens"]["account_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                });
            serde_json::to_vec(&Binding {
                runner: "codex".into(),
                account_id,
                codex_home,
                program: session
                    .agent_program
                    .clone()
                    .unwrap_or_else(|| "codex".into()),
                resume_id: resume.map(str::to_owned),
            })?
        }
        Err(error) => return Err(error),
    };
    Ok(GeneratedFile { path, contents })
}
