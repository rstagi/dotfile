//! Session places with a fallback for workspace window records.

use crate::sessions::{Result, Sessions};
use std::path::{Path, PathBuf};
use ws_core::registry::Session;

pub(crate) enum Place {
    Workspace {
        name: String,
        directory: PathBuf,
    },
    // The directory start endpoint will construct this variant.
    #[allow(dead_code)]
    Directory {
        directory: PathBuf,
    },
}

impl Place {
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
