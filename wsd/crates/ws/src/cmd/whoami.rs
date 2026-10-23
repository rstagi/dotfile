use std::{env, path::Path};

use ws_core::registry::Session;
use ws_daemon::tmux::Tmux;

use crate::{Result, client};

const OUTSIDE: &str = "whoami: not inside a ws session";

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let session = current(socket).await?;
    if json {
        println!("{}", super::session_json(&session)?);
    } else {
        println!("#{}", session.alias);
    }
    Ok(())
}

pub async fn current(socket: &Path) -> Result<Session> {
    let target = identity().ok_or(OUTSIDE)?;
    let target = if target.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("#{target}")
    } else {
        target
    };
    client::session(socket, &target).await.map_err(|error| {
        if client::is_missing(error.as_ref()) {
            OUTSIDE.into()
        } else {
            error
        }
    })
}

pub(super) fn identity() -> Option<String> {
    for key in ["WS_SESSION_ID", "WS_SESSION"] {
        if let Ok(value) = env::var(key)
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    let target = env::var("TMUX_PANE")
        .ok()
        .filter(|pane| !pane.is_empty())
        .or_else(|| {
            env::var_os("TMUX")
                .filter(|tmux| !tmux.is_empty())
                .map(|_| ":".into())
        })?;
    Tmux::from_env()
        .window_option(&target, "@ws_session")
        .ok()
        .flatten()
        .filter(|identity| !identity.is_empty())
}
