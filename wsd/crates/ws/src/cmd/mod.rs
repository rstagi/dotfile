pub mod ack;
pub mod add;
pub mod agent;
pub mod attach;
pub mod build_notifier;
pub mod claude;
pub mod codex;
pub mod dismiss;
pub mod done;
pub mod grok;
pub mod hook;
pub mod inbox;
pub mod list;
pub mod loops;
pub mod mcp;
pub mod merge;
pub mod migrate;
pub mod name;
pub mod new;
pub mod on;
pub mod open;
pub mod phase;
pub mod pick;
pub mod picker;
pub mod pin;
pub mod rm;
pub mod send;
pub mod session;
pub mod sessions;
pub mod show;
pub mod top;
pub mod triggers;
pub mod wait;
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
