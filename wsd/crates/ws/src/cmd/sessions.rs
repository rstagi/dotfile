use std::{path::Path, process::ExitCode};

use ws_daemon::tmux::Tmux;
use ws_hub::sessions::HubSession;

use crate::{Result, client};

pub async fn run(
    socket: &Path,
    targets: &[String],
    json: bool,
    all_devices: bool,
) -> Result<ExitCode> {
    if targets.len() > 1 {
        return Err("sessions: expected [id|name]".into());
    }
    if all_devices && !targets.is_empty() {
        return Err("sessions: --all-devices takes no target".into());
    }
    let hub = if all_devices {
        match super::hub::client()? {
            Some(hub) => Some(hub),
            None => return Ok(super::hub::no_hub()),
        }
    } else {
        None
    };
    let sessions = match targets.first() {
        Some(target) if target == "me" => vec![super::whoami::current(socket).await?],
        Some(target) => vec![client::session(socket, target).await?],
        None => client::sessions(socket).await?,
    };
    let remote = match hub {
        Some(hub) => remote_sessions(&hub, &sessions).await?,
        None => Vec::new(),
    };
    if json {
        let mut rows = sessions
            .iter()
            .map(super::session_json)
            .collect::<serde_json::Result<Vec<_>>>()?;
        for session in &remote {
            let mut row = serde_json::to_value(session)?;
            row["remote"] = true.into();
            rows.push(row);
        }
        println!("{}", serde_json::to_string(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }
    let windows = Tmux::from_env().session_states().unwrap_or_default();
    let parents: Vec<_> = sessions
        .iter()
        .map(|session| {
            let parent = super::parent(session)?;
            sessions.iter().position(|other| other.id == parent)
        })
        .collect();
    let created: Vec<_> = sessions.iter().map(|session| session.alias).collect();
    for (index, depth) in super::nest(&parents, &created) {
        let session = &sessions[index];
        let label = super::tree_prefix(depth) + &super::label(session);
        let alias = format!("#{}", session.alias);
        let state = windows
            .iter()
            .find(|(identity, _)| matches_identity(session, identity))
            .map_or("stopped", |(_, state)| state.as_str());
        let fields = [
            label.as_str(),
            alias.as_str(),
            session.workspace_id.as_deref().unwrap_or(""),
            super::agent(session),
            state,
            super::status(session),
        ];
        print_row(fields);
    }
    for session in &remote {
        // A remote session has no local alias, so the second column holds the ULID.
        print_row([
            session.name.as_deref().unwrap_or(&session.id),
            &session.id,
            &session.device_name,
            &session.kind,
            &session.state,
            session.cwd.as_deref().unwrap_or(""),
        ]);
    }
    Ok(ExitCode::SUCCESS)
}

/// The sessions of the other devices. The hub table also holds the sessions of this device.
async fn remote_sessions(
    hub: &ws_hub::client::HubClient,
    local: &[ws_core::registry::Session],
) -> Result<Vec<HubSession>> {
    let response = hub.request(hyper::Method::GET, "/v1/sessions").await?;
    if response.status != 200 {
        return Err(format!("The hub answered {}: {}", response.status, response.body).into());
    }
    let mut rows: Vec<HubSession> = serde_json::from_str(&response.body)?;
    rows.retain(|row| !local.iter().any(|session| session.id.to_string() == row.id));
    Ok(rows)
}

fn print_row(fields: [&str; 6]) {
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

pub(super) fn matches_identity(session: &ws_core::registry::Session, identity: &str) -> bool {
    identity == session.id.to_string()
        || identity.strip_prefix('#').unwrap_or(identity) == session.alias.to_string()
}
