pub mod add;
pub mod agent;
pub mod attach;
pub mod claude;
pub mod codex;
pub mod grok;
pub mod list;
pub mod merge;
pub mod name;
pub mod new;
pub mod open;
pub mod pin;
pub mod rm;
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

async fn caller(socket: &std::path::Path) -> crate::Result<Option<ws_core::ids::SessionId>> {
    if whoami::identity().is_some() {
        Ok(Some(whoami::current(socket).await?.id))
    } else {
        Ok(None)
    }
}
