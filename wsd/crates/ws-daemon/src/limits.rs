//! Session spawn limits, checked while the lifecycle store is locked.

use crate::sessions::{Error, Result, Sessions};
use std::{collections::HashSet, fmt};
use ws_core::ids::SessionId;

#[derive(Debug)]
pub enum SpawnLimit {
    Children { parent: SessionId, limit: usize },
    Depth { parent: SessionId, limit: usize },
}

impl Sessions {
    pub(crate) fn check_spawn_limits(&self, parent: Option<SessionId>) -> Result<()> {
        let Some(parent) = parent else {
            return Ok(());
        };
        self.get_record(&parent.to_string())?;
        let sessions = self.registry.sessions()?;
        let limit = self.config.limits.max_children;
        let parent_id = parent.to_string();
        let count = sessions
            .iter()
            .filter(|session| session.body["parent_session_id"].as_str() == Some(&parent_id))
            .count();
        if count >= limit {
            return Err(Error::SpawnLimit(SpawnLimit::Children { parent, limit }));
        }
        let limit = self.config.limits.max_depth;
        let mut ancestor = Some(parent);
        let mut depth = 0;
        let mut visited = HashSet::new();
        while let Some(id) = ancestor {
            if depth >= limit {
                return Err(Error::SpawnLimit(SpawnLimit::Depth { parent, limit }));
            }
            if !visited.insert(id) {
                return Err(Error::InvalidRequest(
                    "The session ancestry contains a cycle.",
                ));
            }
            let session = sessions
                .iter()
                .find(|session| session.id == id)
                .ok_or_else(|| ws_core::registry::Error::NotFound(id.to_string()))?;
            ancestor = session.body["parent_session_id"]
                .as_str()
                .map(str::parse::<SessionId>)
                .transpose()
                .map_err(|_| {
                    Error::InvalidRequest("The session ancestry contains an invalid id.")
                })?;
            depth += 1;
        }
        Ok(())
    }
}

impl fmt::Display for SpawnLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Depth { parent, limit } => write!(
                formatter,
                "A child of session {parent} exceeds the depth limit of {limit}."
            ),
            Self::Children { parent, limit } => write!(
                formatter,
                "Session {parent} reached its child limit of {limit}."
            ),
        }
    }
}

impl std::error::Error for SpawnLimit {}
