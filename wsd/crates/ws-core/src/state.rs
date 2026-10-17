//! Session lifecycle states and legal transitions.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Created,
    Running,
    Blocked,
    Finished,
    Failed,
    Reopened,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionError {
    pub from: SessionState,
    pub to: SessionState,
}

impl SessionState {
    /// Validates a transition. Pending launches can fail or finish after a stop request.
    pub fn transition(self, to: Self) -> Result<Self, TransitionError> {
        use SessionState::*;
        match (self, to) {
            (Created | Reopened, Running | Finished | Failed)
            | (Running, Blocked | Finished | Failed)
            | (Blocked, Running | Finished | Failed)
            | (Finished | Failed, Reopened) => Ok(to),
            _ => Err(TransitionError { from: self, to }),
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::Created => "session.created",
            Self::Running => "session.running",
            Self::Blocked => "session.blocked",
            Self::Finished => "session.finished",
            Self::Failed => "session.failed",
            Self::Reopened => "session.reopened",
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.event_type().strip_prefix("session.").unwrap())
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Cannot change the session state from {} to {}.",
            self.from, self.to
        )
    }
}

impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::{SessionState::*, *};

    #[test]
    fn validates_every_state_pair_and_returns_a_typed_error() {
        let states = [Created, Running, Blocked, Finished, Failed, Reopened];
        let allowed = [
            (Created, Running),
            (Created, Finished), // Stop before launch.
            (Created, Failed),   // Launch failure.
            (Running, Blocked),
            (Running, Finished),
            (Running, Failed),
            (Blocked, Running),
            (Blocked, Finished),
            (Blocked, Failed),
            (Finished, Reopened),
            (Failed, Reopened),
            (Reopened, Running),
            (Reopened, Finished), // Stop before relaunch.
            (Reopened, Failed),   // Relaunch failure.
        ];
        for from in states {
            for to in states {
                let result = from.transition(to);
                if allowed.contains(&(from, to)) {
                    assert_eq!(result, Ok(to));
                } else {
                    assert_eq!(result, Err(TransitionError { from, to }));
                }
            }
        }
    }
}
