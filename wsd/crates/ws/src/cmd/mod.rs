pub mod add;
pub mod list;
pub mod name;
pub mod new;
pub mod sessions;
pub mod show;
pub mod whoami;

use ws_core::registry::Session;

pub fn agent(session: &Session) -> &str {
    session
        .body
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

pub fn status(session: &Session) -> &str {
    match session.state.as_str() {
        "finished" => "done",
        "failed" => "closed",
        _ => "active",
    }
}
