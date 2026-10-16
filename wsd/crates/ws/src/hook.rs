use std::{env, path::Path};

use ws_daemon::tmux::Tmux;

use crate::{Result, client};

pub async fn stop(socket: &Path) -> Result<()> {
    let pane = env::var("TMUX_PANE").map_err(|_| "Run `ws hook stop` inside a tmux pane.")?;
    if !pane
        .strip_prefix('%')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("TMUX_PANE must contain a tmux pane id such as %1.".into());
    }
    if env::var_os("TMUX").is_none() && env::var_os("WS_TMUX_SOCKET").is_none() {
        return Err("Set TMUX or WS_TMUX_SOCKET to locate the tmux server.".into());
    }
    let window =
        tokio::task::spawn_blocking(move || Tmux::from_env().window_for_pane(&pane)).await??;
    client::hook_stop(socket, window).await
}
