//! Interactive directory starts and session place records.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};
use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use ws_core::{ids::SessionId, registry::Session};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartSession {
    cwd: Option<PathBuf>,
    #[serde(default)]
    scratch: bool,
    agent: String,
    name: Option<String>,
    prompt: Option<String>,
    parent_session_id: Option<SessionId>,
}

pub(crate) struct StartOptions {
    pub id: SessionId,
    pub name: Option<String>,
    pub prompt: Option<String>,
}

pub(crate) enum Place {
    Workspace { name: String, directory: PathBuf },
    Directory { directory: PathBuf },
}

pub(crate) async fn start(
    State(store): State<Store>,
    Json(request): Json<StartSession>,
) -> std::result::Result<(StatusCode, Json<Value>), ApiError> {
    let (place, agent, parent, options) = api::call(store.clone(), move |sessions| {
        sessions.record_call(request.parent_session_id, "session.start", None)?;
        if !["claude", "codex", "grok"].contains(&request.agent.as_str()) {
            return Err(Error::InvalidRequest(
                "Use the agent claude, codex or grok.",
            ));
        }
        if request.cwd.is_some() == request.scratch {
            return Err(Error::InvalidRequest("Supply either cwd or scratch: true."));
        }
        let options = StartOptions {
            id: SessionId::new(),
            name: request.name,
            prompt: request.prompt,
        };
        options.validate(sessions)?;
        sessions.check_spawn_limits(request.parent_session_id)?;
        let directory = match request.cwd {
            Some(cwd) => cwd
                .canonicalize()
                .map_err(|_| Error::InvalidRequest("Cannot resolve the requested directory."))?,
            None => {
                let ws_home = env::var_os("WS_HOME")
                    .map(PathBuf::from)
                    .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws")))
                    .ok_or(Error::InvalidRequest(
                        "Set WS_HOME or HOME to locate scratch directories.",
                    ))?;
                let id = options.id.to_string();
                let suffix = &id[id.len() - 8..];
                let leaf = options
                    .name
                    .as_ref()
                    .map_or_else(|| suffix.to_owned(), |name| format!("{name}-{suffix}"));
                let root = ws_home.join("scratch");
                fs::create_dir_all(&root)?;
                let directory = root.join(leaf);
                fs::create_dir(&directory)?;
                directory.canonicalize()?
            }
        };
        if !directory.is_dir() || directory.to_str().is_none() {
            return Err(Error::InvalidRequest("Use a directory with a UTF-8 path."));
        }
        Ok((
            Place::Directory { directory },
            request.agent,
            request.parent_session_id,
            options,
        ))
    })
    .await?;
    let launched =
        crate::workspaces::start_agent_with_options(store, place, agent, parent, Some(options))
            .await?;
    Ok((StatusCode::CREATED, Json(launched)))
}

impl StartOptions {
    pub(crate) fn validate(&self, sessions: &Sessions) -> Result<()> {
        if let Some(name) = &self.name {
            if name == "me"
                || !name
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphabetic())
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
            {
                return Err(Error::InvalidRequest(
                    "Start the name with a letter. Use letters, digits, dots, underscores or hyphens. The name me is reserved.",
                ));
            }
            if sessions
                .registry
                .sessions()?
                .iter()
                .any(|session| session.name.as_ref() == Some(name))
            {
                return Err(Error::NameTaken(name.clone()));
            }
        }
        Ok(())
    }
}

impl Place {
    pub(crate) fn tmux_session_name(&self) -> String {
        match self {
            Self::Workspace { name, .. } => name.replace(['.', ':'], "_"),
            Self::Directory { directory } => {
                let basename = directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("root");
                let digest = Sha256::digest(directory.as_os_str().as_encoded_bytes());
                format!(
                    "{}-{:02x}{:02x}{:02x}",
                    basename.replace(['.', ':'], "_"),
                    digest[0],
                    digest[1],
                    digest[2]
                )
            }
        }
    }

    pub(crate) fn directory(&self) -> &Path {
        match self {
            Self::Workspace { directory, .. } | Self::Directory { directory } => directory,
        }
    }

    pub(crate) fn workspace_id(&self) -> Option<&str> {
        match self {
            Self::Workspace { name, .. } => Some(name),
            Self::Directory { .. } => None,
        }
    }
}

impl Sessions {
    /// Returns the current session window, or its legacy workspace windows.
    pub(crate) fn session_windows(&self, session: &Session) -> Result<Vec<String>> {
        if let Some(event) = self.registry.latest_event(session.id, "session.window")? {
            return Ok(event.body["window_id"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .collect());
        }
        let id = session.id.to_string();
        Ok(self
            .registry
            .workspaces()?
            .into_iter()
            .flat_map(|workspace| {
                workspace.body["windows"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter(|window| window["session_id"] == id)
            .filter_map(|window| window["window_id"].as_str().map(str::to_owned))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::Place;
    use std::path::Path;

    #[test]
    fn workspace_keeps_its_identity_separate_from_its_directory() {
        let place = Place::Workspace {
            name: "api/feat-login".into(),
            directory: "/tmp/worktrees/api/feat-login".into(),
        };
        assert_eq!(place.workspace_id(), Some("api/feat-login"));
        assert_eq!(
            place.directory(),
            Path::new("/tmp/worktrees/api/feat-login")
        );
    }

    #[test]
    fn directory_has_no_workspace_identity() {
        let place = Place::Directory {
            directory: "/tmp/notes".into(),
        };
        assert_eq!(place.workspace_id(), None);
        assert_eq!(place.directory(), Path::new("/tmp/notes"));
    }
}
