use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use super::list::{self, Row};
use crate::Result;

pub async fn rows(socket: &Path) -> Result<()> {
    let mut places: BTreeMap<(bool, String), Vec<Row>> = BTreeMap::new();
    for row in list::rows(socket).await? {
        places
            .entry((row.scratch, row.path.clone()))
            .or_default()
            .push(row);
    }
    let mut groups: Vec<_> = places.into_values().collect();
    // Pinned workspaces come first, then workspaces, then directory places.
    groups.sort_by_key(|rows| {
        let first = &rows[0];
        (
            first.scratch,
            !first.pinned,
            if first.scratch {
                first.path.clone()
            } else {
                first.name.split('#').next().unwrap().to_owned()
            },
        )
    });
    // A place nests under the place of the session that created its first session.
    let anchors: Vec<_> = groups
        .iter()
        .map(|rows| {
            rows.iter()
                .filter(|row| row.session.is_some())
                .min_by_key(|row| row.session)
        })
        .collect();
    let parents: Vec<_> = anchors
        .iter()
        .map(|anchor| {
            let parent = anchor.and_then(|row| row.parent)?;
            groups
                .iter()
                .position(|rows| rows.iter().any(|row| row.session == Some(parent)))
        })
        .collect();
    let created: Vec<_> = anchors
        .iter()
        .map(|anchor| anchor.and_then(|row| row.session))
        .collect();
    let expanded = sessions_marker()?.exists();
    let home = std::env::var("HOME").ok();
    let mut previous = String::new();
    let mut heading = false;
    for (index, depth) in super::nest(&parents, &created) {
        let sessions = &groups[index];
        let first = &sessions[0];
        let (scratch, pinned) = (first.scratch, first.pinned);
        if scratch && depth == 0 && !heading {
            println!("\x1b[1mscratch\x1b[0m\t\t");
            heading = true;
        }
        let name = first.name.split('#').next().unwrap();
        let title;
        let (repo, branch) = if scratch {
            let scratch_dir = ws_daemon::workspaces::is_scratch_directory(Path::new(&first.path))
                .unwrap_or(false);
            title = place_title(
                &first.label,
                &first.name,
                &first.path,
                scratch_dir,
                home.as_deref(),
            );
            (first.name.as_str(), title.as_str())
        } else {
            name.split_once('/').unwrap_or((name, name))
        };
        let style = if pinned || repo != previous {
            "\x1b[1m"
        } else {
            "\x1b[2m"
        };
        let mark = if pinned { "📌" } else { "  " };
        let tree = super::tree_prefix(depth);
        let mut state = first.state.as_str();
        let mut target = first.window.as_str();
        let mut unread = first.unread;
        let mut active = first.active;
        for session in &sessions[1..] {
            if rank(&session.state) > rank(state) {
                state = &session.state;
            }
            if !unread && (session.unread || (session.active && !active)) {
                target = &session.window;
            }
            active |= session.active;
            unread |= session.unread;
        }
        let detail = if sessions.len() > 1 {
            format!("{} sessions", sessions.len())
        } else if first.agent == "-" {
            state.into()
        } else {
            first.agent.clone()
        };
        println!(
            "{mark} {tree}{style}{repo:<14}\x1b[0m {} {branch:<24} {} \x1b[2m{detail}\x1b[0m\t{}\t{target}",
            unread_mark(unread),
            state_icon(state),
            first.path
        );
        if expanded && sessions.len() > 1 {
            let parents: Vec<_> = sessions
                .iter()
                .map(|session| {
                    let parent = session.parent?;
                    sessions.iter().position(|row| row.session == Some(parent))
                })
                .collect();
            let created: Vec<_> = sessions.iter().map(|session| session.session).collect();
            for (index, nested) in super::nest(&parents, &created) {
                let session = &sessions[index];
                let mut tab = session.agent.clone();
                if let Some((_, slot)) = session.name.rsplit_once('#').filter(|_| !scratch) {
                    tab.push('#');
                    tab.push_str(slot);
                }
                if !session.label.is_empty() {
                    tab.push(' ');
                    tab.push_str(&session.label);
                }
                println!(
                    "                 {} {}{}└ {} {tab:<22} \x1b[2m{}\x1b[0m\t{}\t{}",
                    unread_mark(session.unread),
                    "  ".repeat(depth),
                    "  ".repeat(nested),
                    state_icon(&session.state),
                    session.state,
                    session.path,
                    session.window
                );
            }
        }
        if !pinned {
            previous = repo.into();
        }
    }
    Ok(())
}

/// Returns the title of a directory place: the session name, `scratch` for an
/// unnamed session in a scratch directory, or the path with `~` for the home directory.
fn place_title(
    label: &str,
    alias: &str,
    path: &str,
    scratch_dir: bool,
    home: Option<&str>,
) -> String {
    if label != alias {
        return label.into();
    }
    if scratch_dir {
        return "scratch".into();
    }
    match home.and_then(|home| path.strip_prefix(home)) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.into(),
    }
}

pub async fn summary(socket: &Path) -> Result<()> {
    let rows = list::rows(socket).await?;
    let mut parts = Vec::new();
    for state in ["waiting", "working", "idle", "stopped"] {
        let count = rows.iter().filter(|row| row.state == state).count();
        if count > 0 {
            parts.push(format!("{} {count} {state}", state_icon(state)));
        }
    }
    let unread = rows.iter().filter(|row| row.unread).count();
    if unread > 0 {
        parts.push(format!("\x1b[36m•\x1b[0m {unread} unread"));
    }
    println!(
        "{}",
        if parts.is_empty() {
            "no workspaces".into()
        } else {
            parts.join(" · ")
        }
    );
    println!("\x1b[2m↵ open · ^s sessions · ^p pin · ^u unread · ^x rm\x1b[0m");
    println!("\x1b[2m^n/^o/^r scratch claude/codex/grok · alt-n/o/r new claude/codex/grok\x1b[0m");
    println!("\x1b[2m^a/^t/^v add claude/codex/grok tab\x1b[0m");
    Ok(())
}

pub async fn preview(socket: &Path, workspace: &str, window: &str) -> Result<ExitCode> {
    if workspace.is_empty() && window.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    let workspaces = list::workspaces(socket).await?;
    if !window.is_empty() {
        let registered = workspaces.iter().any(|workspace| {
            workspace.body["windows"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|entry| entry["window_id"].as_str() == Some(window))
        });
        let directory_session = if registered {
            false
        } else {
            list::rows(socket)
                .await?
                .iter()
                .any(|row| row.scratch && row.window == window)
        };
        if !registered && !directory_session {
            return Ok(ExitCode::SUCCESS);
        }
        let mut command = ws_daemon::tmux::client();
        if let Some(socket) = std::env::var_os("WS_TMUX_SOCKET").filter(|socket| !socket.is_empty())
        {
            command
                .arg(ws_daemon::tmux::socket_flag(&socket))
                .arg(socket);
        }
        return Ok(exit_code(
            command
                .args(["capture-pane", "-e", "-p", "-t", window])
                .status()?,
        ));
    }
    if workspaces
        .iter()
        .any(|entry| entry.body["path"].as_str() == Some(workspace))
        && Path::new(workspace).is_dir()
    {
        println!("stopped — enter restores its sessions");
        // Flush the header before the child writes to the same output stream.
        use std::io::Write;
        std::io::stdout().flush()?;
        Command::new("git")
            .args(["-C", workspace, "log", "--oneline", "-10", "--color=always"])
            .status()?;
        return Ok(exit_code(
            Command::new("git")
                .args(["-C", workspace, "status", "-s"])
                .status()?,
        ));
    }
    if Path::new(workspace).is_dir()
        && list::rows(socket)
            .await?
            .iter()
            .any(|row| row.scratch && row.path == workspace)
    {
        println!("stopped — enter restores its sessions");
        let mut entries = std::fs::read_dir(workspace)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        for entry in entries {
            println!("{}", entry.to_string_lossy());
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn toggle_sessions() -> Result<()> {
    let marker = sessions_marker()?;
    std::fs::create_dir_all(marker.parent().unwrap())?;
    if marker.exists() {
        std::fs::remove_file(marker)?;
    } else {
        std::fs::File::create(marker)?;
    }
    Ok(())
}

pub fn toggle_unread(window: &str) -> Result<()> {
    if window.is_empty() {
        return Ok(());
    }
    let tmux = ws_daemon::tmux::Tmux::from_env();
    if tmux
        .window_option(window, "@ws_unread")?
        .is_some_and(|value| !value.is_empty())
    {
        tmux.unset_window_option(window, "@ws_unread")?;
    } else {
        tmux.set_window_option(window, "@ws_unread", "1")?;
    }
    Ok(())
}

fn sessions_marker() -> Result<PathBuf> {
    let home = match std::env::var_os("WS_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from(
            std::env::var_os("HOME").ok_or("Set HOME or WS_HOME to locate picker preferences.")?,
        )
        .join(".ws"),
    };
    Ok(home.join("show-sessions"))
}

fn unread_mark(unread: bool) -> &'static str {
    if unread { "\x1b[1;36m•\x1b[0m" } else { " " }
}

fn state_icon(state: &str) -> &'static str {
    match state {
        "waiting" => "\x1b[31m●\x1b[0m",
        "working" => "\x1b[33m◐\x1b[0m",
        "idle" => "\x1b[32m○\x1b[0m",
        _ => "\x1b[2m·\x1b[0m",
    }
}

fn rank(state: &str) -> u8 {
    match state {
        "waiting" => 3,
        "working" => 2,
        "idle" => 1,
        _ => 0,
    }
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1) as u8)
}

#[cfg(test)]
mod tests {
    use super::place_title;

    #[test]
    fn place_title_prefers_the_session_name() {
        let title = place_title("fix-login", "#208", "/u/.ws/scratch/AB12", true, Some("/u"));
        assert_eq!(title, "fix-login");
    }

    #[test]
    fn place_title_names_an_unnamed_scratch_session_scratch() {
        let title = place_title("#208", "#208", "/u/.ws/scratch/AB12", true, Some("/u"));
        assert_eq!(title, "scratch");
    }

    #[test]
    fn place_title_shortens_a_directory_under_home() {
        assert_eq!(
            place_title("#9", "#9", "/u/Dev/app", false, Some("/u")),
            "~/Dev/app"
        );
        assert_eq!(place_title("#9", "#9", "/u", false, Some("/u")), "~");
        assert_eq!(
            place_title("#9", "#9", "/user2/x", false, Some("/u")),
            "/user2/x"
        );
        assert_eq!(place_title("#9", "#9", "/srv/x", false, None), "/srv/x");
    }
}
