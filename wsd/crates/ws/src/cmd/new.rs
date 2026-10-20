use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

#[derive(clap::Args)]
pub struct Options {
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub branch: Option<String>,
    /// Start in an existing directory.
    #[arg(long, conflicts_with_all = ["repo", "branch", "scratch"])]
    pub cwd: Option<String>,
    /// Start in a new scratch directory.
    #[arg(long, conflicts_with_all = ["repo", "branch"])]
    pub scratch: bool,
    #[arg(long, default_value = "claude")]
    pub agent: String,
    #[arg(long)]
    pub detach: bool,
}

pub async fn run(socket: &Path, options: Options) -> Result<()> {
    let Options {
        repo,
        branch,
        scratch,
        cwd,
        agent,
        detach,
    } = options;
    if scratch {
        return start_directory(socket, json!({"scratch": true}), agent, detach).await;
    }
    if let Some(cwd) = cwd {
        let cwd = expand_home(&cwd)?
            .canonicalize()
            .map_err(|_| format!("Cannot resolve the directory: {cwd}"))?;
        return start_directory(socket, json!({"cwd": cwd}), agent, detach).await;
    }
    let repo = match repo {
        Some(repo) => Some(expand_home(&repo)?),
        None => pick_repo()?,
    };
    let Some(repo) = repo else {
        if branch.is_some() {
            return Err("Use --branch with a git repository.".into());
        }
        return start_directory(socket, json!({"scratch": true}), agent, detach).await;
    };
    let repo = repo
        .canonicalize()
        .map_err(|_| format!("not a git repo: {}", repo.display()))?;
    if !std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?
        .status
        .success()
    {
        if branch.is_some() {
            return Err("Use --branch with a git repository.".into());
        }
        return start_directory(socket, json!({"cwd": repo}), agent, detach).await;
    }
    let branch = match branch {
        Some(branch) => branch,
        None => {
            use std::io::{BufRead, Write};
            let mut tty = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")?;
            write!(
                tty,
                "branch ({}): ",
                repo.file_name().unwrap().to_string_lossy()
            )?;
            tty.flush()?;
            let mut branch = String::new();
            std::io::BufReader::new(tty).read_line(&mut branch)?;
            branch.trim_end().to_owned()
        }
    };
    if branch.is_empty() {
        return Err("new: branch required".into());
    }
    let parent_session_id = super::caller(socket).await?;
    let body = client::request(
        socket,
        Method::POST,
        "/v1/workspaces",
        serde_json::to_vec(&json!({"repo": repo, "branch": branch, "agent": agent, "parent_session_id": parent_session_id}))?,
    )
    .await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    focus(
        response["window_id"]
            .as_str()
            .ok_or("The workspace response has no window id.")?,
        detach,
    )
}

pub(super) fn expand_home(path: &str) -> Result<std::path::PathBuf> {
    if path == "~" || path.starts_with("~/") {
        let home = std::env::var_os("HOME").ok_or("Set HOME to expand a repository path.")?;
        Ok(std::path::PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or("")))
    } else {
        Ok(path.into())
    }
}

pub(super) fn focus(window: &str, detach: bool) -> Result<()> {
    super::pick::configure_bindings()?;
    if !detach {
        let tmux = ws_daemon::tmux::Tmux::from_env();
        let session = tmux.execute(&["display-message", "-p", "-t", window, "#{session_id}"])?;
        tmux.execute(&["select-window", "-t", window])?;
        tmux.unset_window_option(window, "@ws_unread")?;
        if std::env::var_os("TMUX").is_some_and(|tmux| !tmux.is_empty()) {
            tmux.execute(&["switch-client", "-t", &session])?;
        } else {
            let mut command = ws_daemon::tmux::client();
            if let Some(socket) =
                std::env::var_os("WS_TMUX_SOCKET").filter(|socket| !socket.is_empty())
            {
                command
                    .arg(ws_daemon::tmux::socket_flag(&socket))
                    .arg(socket);
            }
            if !command
                .args(["attach-session", "-t", &session])
                .status()?
                .success()
            {
                return Err("Cannot attach to the workspace.".into());
            }
        }
    }
    Ok(())
}

async fn start_directory(
    socket: &Path,
    mut body: serde_json::Value,
    agent: String,
    detach: bool,
) -> Result<()> {
    body["agent"] = json!(agent);
    body["parent_session_id"] = json!(super::caller(socket).await?);
    let response = client::request(
        socket,
        Method::POST,
        "/v1/sessions/start",
        serde_json::to_vec(&body)?,
    )
    .await?;
    let response: serde_json::Value = serde_json::from_slice(&response)?;
    focus(
        response["window_id"]
            .as_str()
            .ok_or("The session response has no window id.")?,
        detach,
    )
}

fn pick_repo() -> Result<Option<std::path::PathBuf>> {
    use std::{
        env,
        io::Write,
        process::{Command, Stdio},
    };
    let home = std::path::PathBuf::from(
        env::var_os("HOME").ok_or("Set HOME to locate repository candidates.")?,
    );
    let mut candidates = Vec::new();
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        && output.status.success()
    {
        let common = std::path::PathBuf::from(String::from_utf8(output.stdout)?.trim());
        if common.file_name().is_some_and(|name| name == ".git") {
            candidates.push(common.parent().unwrap().to_owned());
        }
    }
    let registry = env::var_os("LOOP_REPO_REGISTRY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".loop/repos.json"));
    match std::fs::read(registry) {
        Ok(bytes) => {
            let mappings: serde_json::Map<String, serde_json::Value> =
                serde_json::from_slice(&bytes)?;
            candidates.extend(
                mappings
                    .values()
                    .filter_map(|value| value.as_str())
                    .map(std::path::PathBuf::from),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let roots = env::var("WS_REPO_ROOTS")
        .map(|roots| shell_words::split(&roots))
        .unwrap_or_else(|_| {
            Ok(vec![
                home.join("Dev").to_string_lossy().into_owned(),
                home.join("dotfile").to_string_lossy().into_owned(),
            ])
        })?;
    for root in roots {
        scan_repos(&expand_home(&root)?, 0, &mut candidates);
    }
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|path| path.is_dir() && seen.insert(path.clone()));
    let mut child = Command::new("fzf")
        .args([
            "--prompt",
            "repo (or type a path)> ",
            "--no-sort",
            "--print-query",
            "--bind",
            "enter:accept-or-print-query",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "fzf not installed (or pass --repo)")?;
    let written = (|| -> std::io::Result<()> {
        let mut input = child.stdin.take().unwrap();
        writeln!(input, "scratch")?;
        for path in candidates {
            writeln!(input, "{}", path.display())?;
        }
        Ok(())
    })();
    let output = child.wait_with_output()?;
    written?;
    let text = String::from_utf8(output.stdout)?;
    let selected = text
        .lines()
        .last()
        .filter(|line| !line.is_empty())
        .ok_or("new: repository selection is empty")?;
    if selected == "scratch" {
        Ok(None)
    } else {
        Ok(Some(expand_home(selected)?))
    }
}

fn scan_repos(root: &Path, depth: usize, candidates: &mut Vec<std::path::PathBuf>) {
    if !root.is_dir() {
        return;
    }
    if root.join(".git").is_dir() {
        candidates.push(root.to_owned());
        return;
    }
    if depth >= 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut paths = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if path.file_name().is_some_and(|name| {
            ["node_modules", ".venv", "vendor"]
                .iter()
                .any(|skip| name == *skip)
        }) {
            continue;
        }
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir())
        {
            scan_repos(&path, depth + 1, candidates);
        }
    }
}
