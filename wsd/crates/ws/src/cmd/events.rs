use std::path::Path;

use serde_json::{Map, Value, json};

use crate::Result;

/// Appends a loop.* event. The daemon records the caller from WS_SESSION_ID.
pub async fn event(
    socket: &Path,
    target: &str,
    event_type: &str,
    body: Option<&str>,
    source: Option<&str>,
) -> Result<()> {
    let body: Option<Map<String, Value>> = body
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| "Use a JSON object for --body.")?;
    super::mcp::print(
        socket,
        "session_event",
        json!({"target":target,"type":event_type,"body":body,"source":source}),
    )
    .await
}

pub async fn log(
    socket: &Path,
    target: &str,
    event_type: Option<String>,
    since: Option<u64>,
) -> Result<()> {
    super::mcp::print(
        socket,
        "session_log",
        json!({"target":target,"type":event_type,"since":since}),
    )
    .await
}
