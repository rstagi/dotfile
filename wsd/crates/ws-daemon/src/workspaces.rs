//! Workspace operations over the durable registry and sandbox-aware tmux driver.

use crate::{
    api::{self, ApiError, CallContext, Store},
    sessions::{Error, Result, Sessions},
    tmux::Tmux,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
use ws_core::{
    ids::{SessionId, Ulid},
    registry::Workspace,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspace {
    pub repo: String,
    pub branch: String,
    pub agent: String,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddWorkspace {
    pub target: String,
    pub agent: String,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Default, Deserialize)]
struct RepoSettings {
    file_include_globs: Option<String>,
    #[serde(default)]
    scripts: RepoScripts,
}

#[derive(Default, Deserialize)]
struct RepoScripts {
    setup: Option<String>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/workspaces", get(list).post(create))
        .route("/v1/workspaces/add", axum::routing::post(add))
}

async fn list(
    State(store): State<Store>,
    Query(context): Query<CallContext>,
) -> std::result::Result<Json<Vec<Workspace>>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(context.parent_session_id, "workspace.list", None)?;
            Ok(sessions.registry.workspaces()?)
        })
        .await?,
    ))
}

async fn create(
    State(store): State<Store>,
    Json(request): Json<CreateWorkspace>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.create_workspace(request)).await?,
    ))
}

async fn add(
    State(store): State<Store>,
    Json(request): Json<AddWorkspace>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.add_workspace(request)).await?,
    ))
}

impl Sessions {
    fn create_workspace(&mut self, request: CreateWorkspace) -> Result<Value> {
        self.record_call(request.parent_session_id, "workspace.create", None)?;
        check_agent(&request.agent)?;
        let repo = main_checkout(Path::new(&request.repo))?;
        let branch = &request.branch;
        if !Command::new("git")
            .args(["check-ref-format", "--branch", branch])
            .output()?
            .status
            .success()
        {
            return Err(failure(format!("invalid branch: {branch}")));
        }
        let settings = repo_settings(&repo)?;
        let root = workspace_root()?;
        let repo_name = repo
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(Error::InvalidRequest("The repository name is invalid."))?;
        let name = format!("{repo_name}/{}", branch.replace('/', "-"));
        let wt = root.join(&name);
        if wt.to_str().is_none() {
            return Err(Error::InvalidRequest("The workspace path is not UTF-8."));
        }
        if wt.try_exists()?
            || self
                .registry
                .workspaces()?
                .iter()
                .any(|workspace| workspace.id == name)
        {
            return Err(failure(format!(
                "workspace exists: {} (use: ws open {name})",
                wt.display()
            )));
        }
        fs::create_dir_all(wt.parent().unwrap())?;
        add_worktree(&repo, branch, &wt)?;
        let result = (|| {
            copy_included_files(&repo, &wt, &settings)?;
            let response =
                self.start_workspace(&name, &wt, &request.agent, request.parent_session_id)?;
            run_setup(
                &repo,
                &wt,
                &settings,
                response["window_id"].as_str().unwrap(),
            )?;
            Ok(response)
        })();
        if result.is_err()
            && !self
                .registry
                .workspaces()?
                .iter()
                .any(|workspace| workspace.id == name)
        {
            let _ = git(
                &repo,
                &["worktree", "remove", "--force", wt.to_str().unwrap()],
            );
        }
        result
    }

    fn add_workspace(&mut self, request: AddWorkspace) -> Result<Value> {
        self.record_call(request.parent_session_id, "workspace.add", None)?;
        check_agent(&request.agent)?;
        let root = workspace_root()?;
        let target = Path::new(&request.target);
        let wt = if target.is_absolute() {
            target.to_owned()
        } else {
            root.join(target)
        };
        let wt = wt
            .canonicalize()
            .map_err(|_| failure(format!("no such workspace: {}", request.target)))?;
        let root = root.canonicalize()?;
        let relative = wt
            .strip_prefix(&root)
            .map_err(|_| failure(format!("no such workspace: {}", request.target)))?;
        if relative.components().count() != 2
            || git(&wt, &["rev-parse", "--show-toplevel"])? != wt.to_str().unwrap()
        {
            return Err(failure(format!("no such workspace: {}", request.target)));
        }
        let name = relative
            .to_str()
            .ok_or(Error::InvalidRequest("The workspace path is not UTF-8."))?;
        self.start_workspace(name, &wt, &request.agent, request.parent_session_id)
    }

    fn start_workspace(
        &mut self,
        name: &str,
        wt: &Path,
        agent: &str,
        parent_session_id: Option<SessionId>,
    ) -> Result<Value> {
        let existing = self
            .registry
            .workspaces()?
            .into_iter()
            .find(|workspace| workspace.id == name);
        let id = Ulid::new();
        let alias = self.registry.next_alias()?;
        let mut windows = existing
            .as_ref()
            .and_then(|workspace| workspace.body["windows"].as_array())
            .cloned()
            .unwrap_or_default();
        let slot = windows
            .iter()
            .filter_map(|window| window["slot"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let tmux = Tmux::from_env();
        let session_name = name.replace(['.', ':'], "_");
        let live_session = if tmux
            .execute(&["has-session", "-t", &format!("={session_name}")])
            .is_ok()
        {
            Some(tmux.execute(&["show-options", "-qv", "-t", &session_name, "@ws_path"])?)
        } else {
            None
        };
        if live_session
            .as_ref()
            .is_some_and(|path| Path::new(path).canonicalize().ok() != wt.canonicalize().ok())
        {
            return Err(failure(format!(
                "The tmux session {session_name} belongs to another workspace."
            )));
        }
        self.append(
            id,
            "session.created",
            json!({"alias": alias, "workspace_id": name, "agent": agent, "slot": slot, "parent_session_id": parent_session_id}),
        )?;
        let tab = if slot > 1 {
            format!("{agent}#{slot}")
        } else {
            agent.into()
        };
        let mode = if live_session.is_some() {
            "new-window"
        } else {
            "new-session"
        };
        let target_flag = if live_session.is_some() { "-t" } else { "-s" };
        let target = if live_session.is_some() {
            format!("{session_name}:")
        } else {
            session_name.clone()
        };
        let mut args = vec![
            mode.to_owned(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            "#{window_id}".into(),
            target_flag.into(),
            target,
            "-n".into(),
            tab,
            "-c".into(),
            wt.to_str().unwrap().into(),
        ];
        for (key, value) in [
            ("WS_WORKSPACE", name.to_owned()),
            ("WS_SESSION", id.to_string()),
            ("WS_SESSION_ID", id.to_string()),
        ] {
            args.extend(["-e".into(), format!("{key}={value}")]);
        }
        for key in [
            "HOME",
            "WS_HOME",
            "WS_ROOT",
            "WS_STATE",
            "WS_SOCK",
            "WS_TMUX_SOCKET",
        ] {
            if let Ok(value) = env::var(key) {
                args.extend(["-e".into(), format!("{key}={value}")]);
            }
        }
        let window = match tmux.execute(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
            Ok(window) => window,
            Err(error) => {
                self.transition(
                    &id.to_string(),
                    ws_core::state::SessionState::Failed,
                    parent_session_id,
                )?;
                return Err(error.into());
            }
        };
        let result = (|| {
            for (key, value) in [
                ("@ws_path", wt.to_str().unwrap()),
                ("@ws_slot", &slot.to_string()),
                ("@ws_agent", agent),
                ("@ws_state", "idle"),
                ("@ws_session", &id.to_string()),
            ] {
                tmux.set_window_option(&window, key, value)?;
            }
            tmux.execute(&[
                "set-option",
                "-t",
                &session_name,
                "@ws_path",
                wt.to_str().unwrap(),
            ])?;
            windows
                .push(json!({"window_id": window, "session_id": id, "agent": agent, "slot": slot}));
            let owner = existing
                .as_ref()
                .map_or(id, |workspace| workspace.owner_session_id);
            let event = if existing.is_some() {
                "workspace.updated"
            } else {
                "workspace.created"
            };
            self.append(owner, event, json!({"workspace_id": name, "path": wt, "tmux_session": session_name, "windows": windows, "parent_session_id": parent_session_id}))?;
            self.transition(
                &id.to_string(),
                ws_core::state::SessionState::Running,
                parent_session_id,
            )?;
            Ok(json!({"window_id": window}))
        })();
        if result.is_err() {
            let _ = tmux.execute(&["kill-window", "-t", &window]);
            let _ = self.transition(
                &id.to_string(),
                ws_core::state::SessionState::Failed,
                parent_session_id,
            );
        }
        result
    }
}

fn workspace_root() -> Result<PathBuf> {
    if let Some(root) = env::var_os("WS_ROOT") {
        return Ok(root.into());
    }
    let home = env::var_os("HOME").ok_or(Error::InvalidRequest(
        "Set HOME or WS_ROOT to locate workspaces.",
    ))?;
    Ok(PathBuf::from(home).join(".ws/worktrees"))
}

fn check_agent(agent: &str) -> Result<()> {
    if ["claude", "codex", "grok"].contains(&agent) {
        Ok(())
    } else {
        Err(Error::InvalidRequest(
            "agent must be one of: claude, codex, grok",
        ))
    }
}

fn main_checkout(repo: &Path) -> Result<PathBuf> {
    let common = git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common);
    if common.file_name().is_none_or(|name| name != ".git") {
        return Err(failure(format!("not a git repo: {}", repo.display())));
    }
    Ok(common.parent().unwrap().to_owned())
}

fn add_worktree(repo: &Path, branch: &str, wt: &Path) -> Result<()> {
    let _ = git(repo, &["fetch", "-q", "origin"]);
    let local = format!("refs/heads/{branch}");
    let remote = format!("refs/remotes/origin/{branch}");
    let path = wt
        .to_str()
        .ok_or(Error::InvalidRequest("The workspace path is not UTF-8."))?;
    if git(repo, &["show-ref", "-q", "--verify", &local]).is_ok() {
        git(repo, &["worktree", "add", "-q", path, branch])?;
    } else if git(repo, &["show-ref", "-q", "--verify", &remote]).is_ok() {
        git(
            repo,
            &[
                "worktree",
                "add",
                "-q",
                "--track",
                "-b",
                branch,
                path,
                &format!("origin/{branch}"),
            ],
        )?;
    } else {
        let base = git(
            repo,
            &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"],
        )
        .unwrap_or_else(|_| "HEAD".into());
        git(repo, &["worktree", "add", "-q", "-b", branch, path, &base])?;
    }
    Ok(())
}

fn copy_included_files(repo: &Path, wt: &Path, settings: &RepoSettings) -> Result<()> {
    let patterns = settings
        .file_include_globs
        .as_deref()
        .unwrap_or(".env*")
        .lines()
        .filter(|line| !line.is_empty())
        .map(glob::Pattern::new)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidRequest("The include pattern is invalid."))?;
    let patterns = if patterns.is_empty() {
        vec![glob::Pattern::new(".env*").unwrap()]
    } else {
        patterns
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
        ])
        .output()?;
    if !output.status.success() {
        return Err(failure("Cannot list ignored repository files."));
    }
    for file in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|file| !file.is_empty())
    {
        let file = std::str::from_utf8(file)
            .map_err(|_| Error::InvalidRequest("The included path is not UTF-8."))?;
        let path = Path::new(file);
        if patterns.iter().any(|pattern| {
            let candidate = if pattern.as_str().contains('/') {
                file
            } else {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
            };
            pattern.matches(candidate)
        }) {
            fs::create_dir_all(wt.join(path).parent().unwrap())?;
            fs::copy(repo.join(path), wt.join(path))?;
        }
    }
    Ok(())
}

fn repo_settings(repo: &Path) -> Result<RepoSettings> {
    let path = repo.join(".conductor/settings.toml");
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|error| failure(format!("Cannot read the repository settings: {error}"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RepoSettings::default()),
        Err(error) => Err(error.into()),
    }
}

fn run_setup(repo: &Path, wt: &Path, settings: &RepoSettings, window: &str) -> Result<()> {
    let script = repo.join(".ws-setup");
    let setup = settings
        .scripts
        .setup
        .as_deref()
        .filter(|setup| !setup.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            fs::metadata(&script)
                .ok()
                .filter(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .map(|_| shell_words::quote(script.to_str().unwrap()).into_owned())
        });
    let Some(setup) = setup else {
        return Ok(());
    };
    let shell = env::var("WS_AGENT_SHELL").unwrap_or_else(|_| "zsh -ic".into());
    let mut command = shell_words::split(&shell)
        .map_err(|_| Error::InvalidRequest("The agent shell command is invalid."))?;
    if command.is_empty() {
        return Err(Error::InvalidRequest("The agent shell command is empty."));
    }
    command.push(format!("{setup} || {{ printf 'setup failed (%s) — enter to close\\n' \"$?\"; read ws_setup_reply; }}"));
    let tmux = Tmux::from_env();
    let session = tmux.execute(&["display-message", "-p", "-t", window, "#{session_id}"])?;
    let mut args = vec![
        "new-window",
        "-d",
        "-t",
        &session,
        "-n",
        "setup",
        "-c",
        wt.to_str().unwrap(),
    ];
    args.extend(command.iter().map(String::as_str));
    tmux.execute(&args)?;
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(failure(format!(
            "git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned())
}

fn failure(message: impl Into<String>) -> Error {
    Error::Io(io::Error::other(message.into()))
}
