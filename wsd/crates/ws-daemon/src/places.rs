//! Session places with a fallback for workspace window records.

use crate::sessions::{Result, Sessions};
use ws_core::registry::Session;

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
