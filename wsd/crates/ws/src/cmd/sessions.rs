use std::path::Path;

use ws_daemon::tmux::Tmux;

use crate::{Result, client};

pub async fn run(socket: &Path, targets: &[String], json: bool) -> Result<()> {
    if targets.len() > 1 {
        return Err("sessions: expected [id|name]".into());
    }
    let sessions = match targets.first() {
        Some(target) if target == "me" => vec![super::whoami::current(socket).await?],
        Some(target) => vec![client::session(socket, target).await?],
        None => client::sessions(socket).await?,
    };
    if json {
        println!("{}", serde_json::to_string(&sessions)?);
        return Ok(());
    }
    let windows = Tmux::from_env().session_states().unwrap_or_default();
    for session in sessions {
        let alias = format!("#{}", session.alias);
        let state = windows
            .iter()
            .find(|(identity, _)| matches_identity(&session, identity))
            .map_or("stopped", |(_, state)| state.as_str());
        let fields = [
            alias.as_str(),
            session.name.as_deref().unwrap_or(""),
            session.workspace_id.as_deref().unwrap_or(""),
            super::agent(&session),
            state,
            super::status(&session),
        ];
        // Match jq's @tsv escaping for embedded control characters.
        println!(
            "{}",
            fields
                .map(|field| field
                    .replace('\\', "\\\\")
                    .replace('\t', "\\t")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r"))
                .join("\t")
        );
    }
    Ok(())
}

pub(super) fn matches_identity(session: &ws_core::registry::Session, identity: &str) -> bool {
    identity == session.id.to_string()
        || identity.strip_prefix('#').unwrap_or(identity) == session.alias.to_string()
}
