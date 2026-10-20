//! Workspace operations over the durable registry and sandbox-aware tmux driver.

use crate::{
    api::{self, ApiError, CallContext, Store},
    places::Place,
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
    env, fs,
    io::{self, Read},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
};
use ws_core::{
    ids::{SessionId, Ulid},
    registry::{Session, Workspace},
};

pub(crate) struct LaunchContext<'a> {
    pub resume: Option<&'a Session>,
    pub run: Option<&'a crate::agents::launch::RunAgent>,
    pub spawn: Option<SpawnOrigin<'a>>,
    pub start: Option<crate::places::StartOptions>,
}

pub(crate) struct SpawnOrigin<'a> {
    pub id: SessionId,
    pub trigger_id: Ulid,
    pub prompt: &'a str,
}

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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTarget {
    pub target: String,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveWorkspace {
    pub target: String,
    #[serde(default)]
    pub confirmed: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub delete_branch: bool,
    #[serde(default)]
    pub force_delete_branch: bool,
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
        .route("/v1/workspaces/pin", axum::routing::post(pin))
        .route("/v1/workspaces/remove", axum::routing::post(remove))
        .route("/v1/workspaces/open", axum::routing::post(open))
}

/// Resolves an explicit worktree under WS_ROOT without registering it.
pub fn workspace_path(target: &str) -> Result<(String, PathBuf)> {
    let root = workspace_root()?;
    let path = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        root.join(target)
    };
    let missing = || failure(format!("no such workspace: {target}"));
    let wt = path.canonicalize().map_err(|_| missing())?;
    let root = root.canonicalize().map_err(|_| missing())?;
    let relative = wt.strip_prefix(&root).map_err(|_| missing())?;
    if relative.components().count() != 2
        || git(&wt, &["rev-parse", "--show-toplevel"]).ok().as_deref() != wt.to_str()
    {
        return Err(missing());
    }
    let name = relative
        .to_str()
        .ok_or(Error::InvalidRequest("The workspace path is not UTF-8."))?
        .to_owned();
    Ok((name, wt))
}

/// Resolves an existing directory that is not a workspace.
pub fn directory_path(target: &str) -> Option<PathBuf> {
    if workspace_path(target).is_ok() {
        return None;
    }
    Path::new(target)
        .canonicalize()
        .ok()
        .filter(|path| path.is_dir())
}

/// Checks the resolved path against the scratch root. The root itself is excluded.
pub fn is_scratch_directory(directory: &Path) -> Result<bool> {
    let home = env::var_os("WS_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws")))
        .ok_or(Error::InvalidRequest(
            "Set WS_HOME or HOME to locate scratch directories.",
        ))?;
    let root = match home.join("scratch").canonicalize() {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let directory = directory.canonicalize()?;
    Ok(directory != root && directory.starts_with(root))
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
    Ok(Json(create_workspace(store, request).await?))
}

/// Names of workspaces whose git and tmux setup runs outside the store lock.
static RESERVED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// Releases a workspace name reservation when creation ends.
struct Reservation(String);

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut reserved) = RESERVED.lock() {
            reserved.remove(&self.0);
        }
    }
}

/// Reserves a workspace name, or fails when another operation holds it.
fn reserve_name(name: &str) -> Result<Reservation> {
    let mut reserved = RESERVED
        .lock()
        .map_err(|_| failure("Cannot lock the workspace reservations."))?;
    if !reserved.insert(name.to_owned()) {
        return Err(failure(format!("workspace busy: {name}")));
    }
    Ok(Reservation(name.to_owned()))
}

struct PlannedWorkspace {
    repo: PathBuf,
    settings: RepoSettings,
    branch: String,
    name: String,
    wt: PathBuf,
}

/// Runs git commands without the store lock. Short lock sections record, reserve and start.
async fn create_workspace(
    store: Store,
    request: CreateWorkspace,
) -> std::result::Result<Value, ApiError> {
    let parent = request.parent_session_id;
    api::call(store.clone(), move |sessions| {
        sessions.record_call(parent, "workspace.create", None)
    })
    .await?;
    let agent = request.agent.clone();
    let plan = off_lock(move || plan_workspace(&request)).await?;
    let (name, wt) = (plan.name.clone(), plan.wt.clone());
    let reservation = api::call(store.clone(), move |sessions| {
        sessions.reserve_workspace(&name, &wt)
    })
    .await?;
    let plan = off_lock(move || {
        fs::create_dir_all(plan.wt.parent().unwrap())?;
        add_worktree(&plan.repo, &plan.branch, &plan.wt)?;
        if let Err(error) = copy_included_files(&plan.repo, &plan.wt, &plan.settings) {
            remove_worktree(&plan.repo, &plan.wt);
            return Err(error);
        }
        Ok(plan)
    })
    .await?;
    let started = start_agent(
        store.clone(),
        Place::Workspace {
            name: plan.name.clone(),
            directory: plan.wt.clone(),
        },
        agent,
        parent,
    )
    .await;
    api::call(store, move |sessions| {
        let _reservation = reservation;
        let PlannedWorkspace {
            repo,
            settings,
            name,
            wt,
            ..
        } = plan;
        let result = started
            .map_err(|ApiError(error)| error)
            .and_then(|response| {
                run_setup(
                    &repo,
                    &wt,
                    &settings,
                    response["window_id"].as_str().unwrap(),
                )?;
                Ok(response)
            });
        if result.is_err()
            && !sessions
                .registry
                .workspaces()?
                .iter()
                .any(|workspace| workspace.id == name)
        {
            remove_worktree(&repo, &wt);
        }
        result
    })
    .await
}

/// Starts a new agent window. The tmux launch runs without the store lock.
pub(crate) async fn start_agent(
    store: Store,
    place: Place,
    agent: String,
    parent: Option<SessionId>,
) -> std::result::Result<Value, ApiError> {
    start_agent_with_options(store, place, agent, parent, None).await
}

pub(crate) async fn start_agent_with_options(
    store: Store,
    place: Place,
    agent: String,
    parent: Option<SessionId>,
    start: Option<crate::places::StartOptions>,
) -> std::result::Result<Value, ApiError> {
    let plan = api::call(store.clone(), move |sessions| {
        sessions.plan_agent(
            place,
            &agent,
            parent,
            LaunchContext {
                resume: None,
                run: None,
                spawn: None,
                start,
            },
        )
    })
    .await?;
    let (plan, launched) = tokio::task::spawn_blocking(move || {
        let launched = plan.launch().and_then(|window| {
            if let Err(error) = wait_for_test_commit_gate() {
                let _ = Tmux::from_env().execute(&["kill-window", "-t", &window]);
                return Err(error);
            }
            Ok(window)
        });
        (plan, launched)
    })
    .await
    .map_err(|_| ApiError(Error::Io(io::Error::other("The workspace worker failed."))))?;
    api::call(store, move |sessions| sessions.commit_agent(plan, launched)).await
}

// The test releases this gate after the pane requests preparation, before the commit.
fn wait_for_test_commit_gate() -> Result<()> {
    if let Some(path) = env::var_os("WS_TEST_AGENT_COMMIT_GATE") {
        let mut gate = UnixStream::connect(path)?;
        gate.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        gate.read_exact(&mut [0])?;
    }
    Ok(())
}

async fn off_lock<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> std::result::Result<T, ApiError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| ApiError(Error::Io(io::Error::other("The workspace worker failed."))))?
        .map_err(ApiError)
}

fn plan_workspace(request: &CreateWorkspace) -> Result<PlannedWorkspace> {
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
    Ok(PlannedWorkspace {
        repo,
        settings,
        branch: branch.clone(),
        name,
        wt,
    })
}

fn remove_worktree(repo: &Path, wt: &Path) {
    let _ = git(
        repo,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
}

async fn add(
    State(store): State<Store>,
    Json(request): Json<AddWorkspace>,
) -> std::result::Result<Json<Value>, ApiError> {
    let agent = request.agent.clone();
    let parent = request.parent_session_id;
    let place = api::call(store.clone(), move |sessions| {
        sessions.record_call(request.parent_session_id, "workspace.add", None)?;
        check_agent(&request.agent)?;
        match workspace_path(&request.target) {
            Ok((name, directory)) => Ok(Place::Workspace { name, directory }),
            Err(error) => {
                let directory = Path::new(&request.target).canonicalize();
                match directory {
                    Ok(directory) if directory.is_dir() && directory.to_str().is_some() => {
                        Ok(Place::Directory { directory })
                    }
                    _ => Err(error),
                }
            }
        }
    })
    .await?;
    Ok(Json(start_agent(store, place, agent, parent).await?))
}

async fn pin(
    State(store): State<Store>,
    Json(request): Json<WorkspaceTarget>,
) -> std::result::Result<Json<Value>, ApiError> {
    let response = api::call(store, move |sessions| {
        sessions.record_call(
            request.parent_session_id,
            "workspace.pin",
            Some(&request.target),
        )?;
        if directory_path(&request.target).is_some() {
            return Err(Error::InvalidRequest(
                "Pins for directory places are not supported.",
            ));
        }
        let workspace = sessions.register_workspace(&request.target, request.parent_session_id)?;
        let pinned = !workspace.body["pinned"].as_bool().unwrap_or(false);
        sessions.append(
            workspace.owner_session_id,
            "workspace.updated",
            json!({
                "workspace_id": workspace.id,
                "pinned": pinned,
                "parent_session_id": request.parent_session_id
            }),
        )?;
        Ok(json!({"pinned": pinned}))
    })
    .await?;
    Ok(Json(response))
}

async fn remove(
    State(store): State<Store>,
    Json(request): Json<RemoveWorkspace>,
) -> std::result::Result<Json<Value>, ApiError> {
    if let Some(directory) = directory_path(&request.target) {
        let delete = is_scratch_directory(&directory).map_err(ApiError)?;
        if delete && !request.confirmed {
            return Err(ApiError(Error::InvalidRequest(
                "Confirm the removal of the scratch directory before deleting it.",
            )));
        }
        let removed_directory = directory.clone();
        let response = api::call(store, move |sessions| {
            sessions.record_call(
                request.parent_session_id,
                "workspace.remove",
                Some(&request.target),
            )?;
            for session in sessions.directory_sessions(&directory)? {
                if !["finished", "failed"].contains(&session.state.as_str()) {
                    sessions.stop(&session.id.to_string(), request.parent_session_id)?;
                }
            }
            Ok(json!({}))
        })
        .await?;
        if delete {
            off_lock(move || {
                if !is_scratch_directory(&removed_directory)? {
                    return Err(Error::InvalidRequest(
                        "The directory is outside the scratch root.",
                    ));
                }
                fs::remove_dir_all(removed_directory)?;
                Ok(())
            })
            .await?;
        }
        return Ok(Json(response));
    }
    let plan = api::call(store.clone(), move |sessions| {
        sessions.begin_remove(request)
    })
    .await?;
    let (plan, removed) = off_lock(move || {
        let removed = remove_files(&plan);
        Ok((plan, removed))
    })
    .await?;
    let removed = removed.map_err(ApiError)?;
    Ok(Json(
        api::call(store, move |sessions| sessions.finish_remove(plan, removed)).await?,
    ))
}

async fn open(
    State(store): State<Store>,
    Json(request): Json<WorkspaceTarget>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| sessions.open_workspace(request)).await?,
    ))
}

impl Sessions {
    fn open_workspace(&mut self, request: WorkspaceTarget) -> Result<Value> {
        self.record_call(
            request.parent_session_id,
            "workspace.open",
            Some(&request.target),
        )?;
        let session = match self.registry.resolve(&request.target) {
            Ok(id) => Some(self.get(&id.to_string(), None)?),
            Err(ws_core::registry::Error::NotFound(_)) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(session) = &session
            && session.workspace_id.is_none()
        {
            let directory = session
                .cwd
                .as_deref()
                .ok_or(Error::InvalidRequest("The session has no directory place."))?;
            return self.open_directory(
                Path::new(directory),
                vec![session.clone()],
                request.parent_session_id,
            );
        }
        if session.is_none()
            && let Some(directory) = directory_path(&request.target)
        {
            let sessions = self.directory_sessions(&directory)?;
            return self.open_directory(&directory, sessions, request.parent_session_id);
        }
        let workspace = if let Some(session) = &session {
            self.registry
                .workspaces()?
                .into_iter()
                .find(|workspace| Some(workspace.id.as_str()) == session.workspace_id.as_deref())
                .ok_or_else(|| {
                    failure(format!(
                        "session workspace no longer exists: {}",
                        session.id
                    ))
                })?
        } else {
            self.register_workspace(&request.target, request.parent_session_id)?
        };
        let mut windows = live_windows(&workspace);
        if let Some(session) = &session {
            windows.retain(|window| window["session_id"].as_str() == Some(&session.id.to_string()));
        }
        windows.sort_by_key(|window| !window["active"].as_bool().unwrap_or(false));
        if let Some(window) = windows.first() {
            return Ok(window.clone());
        }
        let wt = Path::new(
            workspace.body["path"]
                .as_str()
                .ok_or(Error::InvalidRequest("The workspace has no path."))?,
        );
        if !wt.is_dir() {
            return Err(failure(format!(
                "session workspace no longer exists: {}",
                session
                    .as_ref()
                    .map_or_else(|| workspace.id.clone(), |session| session.id.to_string())
            )));
        }
        if let Some(session) = session {
            let agent = session.body["agent"].as_str().unwrap_or("claude");
            return self.start_workspace_session(
                &workspace.id,
                wt,
                agent,
                request.parent_session_id,
                Some(&session),
                None,
            );
        }
        let retained = workspace.body["windows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|window| window["session_id"].as_str())
            .map(|id| self.get(id, None))
            .collect::<Result<Vec<_>>>()?;
        for session in retained
            .into_iter()
            .filter(|session| !["finished", "failed"].contains(&session.state.as_str()))
        {
            let agent = session.body["agent"].as_str().unwrap_or("claude");
            self.start_workspace_session(
                &workspace.id,
                wt,
                agent,
                request.parent_session_id,
                Some(&session),
                None,
            )?;
        }
        let workspace = self.workspace(&workspace.id)?;
        match live_windows(&workspace).first() {
            Some(window) => Ok(window.clone()),
            None => self.start_workspace(&workspace.id, wt, "claude", request.parent_session_id),
        }
    }

    fn directory_sessions(&self, directory: &Path) -> Result<Vec<Session>> {
        Ok(self
            .registry
            .sessions()?
            .into_iter()
            .filter(|session| {
                session.workspace_id.is_none()
                    && session.body["agent"].is_string()
                    && session.cwd.as_deref().is_some_and(|cwd| {
                        Path::new(cwd).canonicalize().ok().as_deref() == Some(directory)
                    })
            })
            .collect())
    }

    fn open_directory(
        &mut self,
        directory: &Path,
        sessions: Vec<Session>,
        parent: Option<SessionId>,
    ) -> Result<Value> {
        if !directory.is_dir() {
            return Err(failure(format!(
                "The session directory no longer exists: {}",
                directory.display()
            )));
        }
        let tmux = Tmux::from_env();
        for session in &sessions {
            for window in self.session_windows(session)? {
                if tmux
                    .window_option(&window, "@ws_session")
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(&session.id.to_string())
                {
                    return Ok(json!({"window_id": window, "session_id": session.id}));
                }
            }
        }
        let mut first = None;
        for session in sessions
            .into_iter()
            .filter(|session| !["finished", "failed"].contains(&session.state.as_str()))
        {
            let window = self.start_place_agent(
                Place::Directory {
                    directory: directory.to_owned(),
                },
                session.body["agent"].as_str().unwrap_or("claude"),
                parent,
                LaunchContext {
                    resume: Some(&session),
                    run: None,
                    spawn: None,
                    start: None,
                },
            )?;
            first.get_or_insert(window);
        }
        first.ok_or_else(|| {
            failure(format!(
                "No stopped session exists in directory: {}",
                directory.display()
            ))
        })
    }

    /// Step 1 of a removal: checks the target and reserves its name under the store lock.
    fn begin_remove(&mut self, request: RemoveWorkspace) -> Result<RemovePlan> {
        self.record_call(
            request.parent_session_id,
            "workspace.remove",
            Some(&request.target),
        )?;
        self.find_workspace(&request.target)?;
        let (name, wt) = workspace_path(&request.target)?;
        let reservation = reserve_name(&name)?;
        let repo = main_checkout(&wt)?;
        let branch = git(&wt, &["branch", "--show-current"])?;
        Ok(RemovePlan {
            request,
            name,
            wt,
            repo,
            branch,
            _reservation: reservation,
        })
    }

    /// Step 3 of a removal: records the result under the store lock and closes the tabs.
    fn finish_remove(&mut self, plan: RemovePlan, removed: Removed) -> Result<Value> {
        let RemovePlan { request, name, .. } = plan;
        let Removed {
            session_ids,
            window_ids,
            warning,
            git_stderr,
        } = removed;
        let tmux = Tmux::from_env();
        let workspace = self
            .registry
            .workspaces()?
            .into_iter()
            .find(|workspace| workspace.id == name);
        if let Some(workspace) = workspace {
            self.append(
                workspace.owner_session_id,
                "workspace.removed",
                json!({
                    "workspace_id": workspace.id, "parent_session_id": request.parent_session_id
                }),
            )?;
        }
        for session in self
            .registry
            .sessions()?
            .into_iter()
            .filter(|session| session.workspace_id.as_deref() == Some(&name))
        {
            if !["finished", "failed"].contains(&session.state.as_str()) {
                self.transition(
                    &session.id.to_string(),
                    ws_core::state::SessionState::Finished,
                    request.parent_session_id,
                )?;
            }
        }
        for id in window_ids {
            let _ = tmux.execute(&["kill-window", "-t", &id]);
        }
        for id in session_ids {
            let _ = tmux.execute(&["kill-session", "-t", &id]);
        }
        Ok(json!({"warning": warning, "git_stderr": git_stderr}))
    }

    fn workspace(&self, target: &str) -> Result<Workspace> {
        self.find_workspace(target)?
            .ok_or_else(|| failure(format!("no such workspace: {target}")))
    }

    fn find_workspace(&self, target: &str) -> Result<Option<Workspace>> {
        let path = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            workspace_root()?.join(target)
        };
        let path = path.canonicalize().ok();
        Ok(self.registry.workspaces()?.into_iter().find(|workspace| {
            path.as_ref().is_some_and(|path| {
                workspace.body["path"]
                    .as_str()
                    .and_then(|stored| Path::new(stored).canonicalize().ok())
                    .as_ref()
                    == Some(path)
            })
        }))
    }

    fn register_workspace(
        &mut self,
        target: &str,
        parent_session_id: Option<SessionId>,
    ) -> Result<Workspace> {
        if let Some(workspace) = self.find_workspace(target)? {
            return Ok(workspace);
        }
        let (name, wt) = workspace_path(target)?;
        let owner = self
            .registry
            .workspace_owner(&name)?
            .unwrap_or_else(Ulid::new);
        self.append(owner, "workspace.created", json!({
            "workspace_id": name, "path": wt, "windows": [], "parent_session_id": parent_session_id
        }))?;
        self.workspace(target)
    }

    fn reserve_workspace(&mut self, name: &str, wt: &Path) -> Result<Reservation> {
        let mut reserved = RESERVED
            .lock()
            .map_err(|_| failure("Cannot lock the workspace reservations."))?;
        if reserved.contains(name)
            || wt.try_exists()?
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
        reserved.insert(name.to_owned());
        Ok(Reservation(name.to_owned()))
    }

    fn start_workspace(
        &mut self,
        name: &str,
        wt: &Path,
        agent: &str,
        parent_session_id: Option<SessionId>,
    ) -> Result<Value> {
        self.start_workspace_session(name, wt, agent, parent_session_id, None, None)
    }

    pub(crate) fn start_workspace_session(
        &mut self,
        name: &str,
        wt: &Path,
        agent: &str,
        parent_session_id: Option<SessionId>,
        resume: Option<&Session>,
        run: Option<&crate::agents::launch::RunAgent>,
    ) -> Result<Value> {
        self.start_workspace_agent(
            name,
            wt,
            agent,
            parent_session_id,
            LaunchContext {
                resume,
                run,
                spawn: None,
                start: None,
            },
        )
    }

    pub(crate) fn start_workspace_agent(
        &mut self,
        name: &str,
        wt: &Path,
        agent: &str,
        parent_session_id: Option<SessionId>,
        context: LaunchContext<'_>,
    ) -> Result<Value> {
        self.start_place_agent(
            Place::Workspace {
                name: name.to_owned(),
                directory: wt.to_owned(),
            },
            agent,
            parent_session_id,
            context,
        )
    }

    pub(crate) fn start_place_agent(
        &mut self,
        place: Place,
        agent: &str,
        parent_session_id: Option<SessionId>,
        context: LaunchContext<'_>,
    ) -> Result<Value> {
        let plan = self.plan_agent(place, agent, parent_session_id, context)?;
        let launched = plan.launch();
        self.commit_agent(plan, launched)
    }

    /// Step 1: checks the request and records the session and slot under the store lock.
    fn plan_agent(
        &mut self,
        place: Place,
        agent: &str,
        parent_session_id: Option<SessionId>,
        context: LaunchContext<'_>,
    ) -> Result<AgentLaunch> {
        let name = place.workspace_id();
        let wt = place.directory();
        let LaunchContext {
            resume,
            run,
            spawn,
            start,
        } = context;
        if resume.is_none() {
            self.check_spawn_limits(parent_session_id)?;
        }
        if let Some(start) = &start {
            start.validate(self)?;
        }
        let existing = self
            .registry
            .workspaces()?
            .into_iter()
            .find(|workspace| Some(workspace.id.as_str()) == name);
        let id = spawn
            .as_ref()
            .map(|origin| origin.id)
            .or_else(|| start.as_ref().map(|start| start.id))
            .unwrap_or_else(|| resume.map_or_else(Ulid::new, |session| session.id));
        let windows = existing
            .as_ref()
            .and_then(|workspace| workspace.body["windows"].as_array())
            .cloned()
            .unwrap_or_default();
        let preferred_slot = resume
            .and_then(|session| {
                windows
                    .iter()
                    .find(|window| window["session_id"].as_str() == Some(&session.id.to_string()))
            })
            .and_then(|window| window["slot"].as_u64());
        let tmux = Tmux::from_env();
        let session_name = if let Some(run) = run {
            tmux.execute(&["display-message", "-p", "-t", &run.pane, "#{session_name}"])?
        } else {
            place.tmux_session_name()
        };
        let live_session = if tmux
            .execute(&["has-session", "-t", &format!("={session_name}")])
            .is_ok()
        {
            Some(tmux.execute(&["show-options", "-qv", "-t", &session_name, "@ws_path"])?)
        } else {
            None
        };
        if run.is_none()
            && live_session
                .as_ref()
                .is_some_and(|path| Path::new(path).canonicalize().ok() != wt.canonicalize().ok())
        {
            let kind = if name.is_some() {
                "workspace"
            } else {
                "directory"
            };
            return Err(failure(format!(
                "The tmux session {session_name} belongs to another {kind}."
            )));
        }
        let pending = crate::agents::launch::PendingLaunch::new(id)?;
        let slot = crate::agents::slots::register(
            &self.state,
            wt,
            preferred_slot,
            agent,
            &id.to_string(),
        )?;
        if let Some(session) = resume {
            if ["finished", "failed"].contains(&session.state.as_str()) {
                self.transition(
                    &id.to_string(),
                    ws_core::state::SessionState::Reopened,
                    parent_session_id,
                )?;
            }
        } else {
            let alias = self.next_alias()?;
            let mut body = json!({"alias": alias, "workspace_id": name, "cwd": wt, "agent": agent, "slot": slot, "parent_session_id": parent_session_id, "arguments": run.map(|r| &r.arguments)});
            if let Some(start) = &start {
                body["name"] = json!(start.name);
                body["prompt"] = json!(start.prompt);
            }
            if let Some(origin) = &spawn {
                body["spawned"] = json!(true);
                body["trigger_id"] = json!(origin.trigger_id);
                body["prompt"] = json!(origin.prompt);
            }
            self.append(id, "session.created", body)?;
        }
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
        if let Some(name) = name {
            args.extend(["-e".into(), format!("WS_WORKSPACE={name}")]);
        }
        args.extend(["-e".into(), format!("WS_SESSION_ID={id}")]);
        for key in [
            "HOME",
            "WS_HOME",
            "WS_ROOT",
            "WS_STATE",
            "WS_SOCK",
            "WS_TMUX_SOCKET",
            "PATH",
            "GROK_HOME",
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "WS_AGENT_SHELL",
        ] {
            if let Ok(value) = env::var(key) {
                args.extend(["-e".into(), format!("{key}={value}")]);
            }
        }
        let ws_bin = env::current_exe()?;
        args.extend(["-e".into(), format!("WS_BIN={}", ws_bin.display())]);
        args.extend([
            "/bin/sh".into(),
            "-c".into(),
            "exec \"$@\"".into(),
            "sh".into(),
            ws_bin.to_string_lossy().into_owned(),
            "_agent".into(),
            id.to_string(),
        ]);
        Ok(AgentLaunch {
            place,
            agent: agent.to_owned(),
            parent_session_id,
            id,
            slot,
            session_name,
            existed: existing.is_some(),
            run_pane: run.map(|run| run.pane.clone()),
            was_running: resume.is_some_and(|session| session.state == "running"),
            pending,
            args,
        })
    }

    /// Step 3: rechecks the session, workspace and slot, then registers the window.
    fn commit_agent(&mut self, plan: AgentLaunch, launched: Result<String>) -> Result<Value> {
        let AgentLaunch {
            place,
            agent,
            parent_session_id,
            id,
            slot,
            session_name,
            existed,
            run_pane,
            was_running,
            pending: _pending,
            args: _,
        } = plan;
        let (name, wt, agent) = (place.workspace_id(), place.directory(), agent.as_str());
        let run = run_pane.as_deref();
        let tmux = Tmux::from_env();
        let window = match launched {
            Ok(window) => window,
            Err(error) => {
                crate::agents::slots::remove(&self.state, wt, &id.to_string())?;
                self.transition(
                    &id.to_string(),
                    ws_core::state::SessionState::Failed,
                    parent_session_id,
                )?;
                return Err(error);
            }
        };
        let existing = self
            .registry
            .workspaces()?
            .into_iter()
            .find(|workspace| Some(workspace.id.as_str()) == name);
        let mut windows = existing
            .as_ref()
            .and_then(|workspace| workspace.body["windows"].as_array())
            .cloned()
            .unwrap_or_default();
        let result = (|| {
            let session = self.get_record(&id.to_string())?;
            if ["finished", "failed"].contains(&session.state.as_str()) {
                return Err(failure(format!(
                    "The session {id} ended during its launch."
                )));
            }
            if let Some(name) = name
                && existed
                && existing.is_none()
            {
                return Err(failure(format!(
                    "The workspace {name} was removed during the launch."
                )));
            }
            if crate::agents::slots::register(&self.state, wt, Some(slot), agent, &id.to_string())?
                != slot
            {
                return Err(failure(format!(
                    "The slot {slot} was reassigned during the launch."
                )));
            }
            let pane = match run {
                Some(pane) => pane.to_owned(),
                None => tmux.execute(&["display-message", "-p", "-t", &window, "#{pane_id}"])?,
            };
            tmux.execute(&[
                "rename-window",
                "-t",
                &window,
                &if slot > 1 {
                    format!("{agent}#{slot}")
                } else {
                    agent.into()
                },
            ])?;
            tmux.set_window_option(&window, "automatic-rename", "off")?;
            for (key, value) in [
                ("@ws_path", wt.to_str().unwrap()),
                ("@ws_slot", &slot.to_string()),
                ("@ws_agent", agent),
                ("@ws_state", "idle"),
                ("@ws_session", &id.to_string()),
                ("@ws_pane", &pane),
            ] {
                tmux.set_window_option(&window, key, value)?;
            }
            if run.is_none() {
                tmux.execute(&[
                    "set-option",
                    "-t",
                    &session_name,
                    "@ws_path",
                    wt.to_str().unwrap(),
                ])?;
            }
            self.append(
                id,
                "session.window",
                json!({"window_id":window, "pane":pane, "slot":slot}),
            )?;
            windows.retain(|window| window["session_id"].as_str() != Some(&id.to_string()));
            windows
                .push(json!({"window_id": window, "session_id": id, "agent": agent, "slot": slot}));
            if let Some(name) = name.filter(|_| session.workspace_id.is_some()) {
                let owner = self.registry.workspace_owner(name)?.unwrap_or(id);
                let event = if existing.is_some() {
                    "workspace.updated"
                } else {
                    "workspace.created"
                };
                self.append(owner, event, json!({"workspace_id": name, "path": wt, "tmux_session": session_name, "windows": windows, "parent_session_id": parent_session_id}))?;
            }
            if !was_running {
                self.transition(
                    &id.to_string(),
                    ws_core::state::SessionState::Running,
                    parent_session_id,
                )?;
            }
            Ok(json!({"window_id": window, "session_id": id}))
        })();
        if result.is_err() {
            let _ = crate::agents::slots::remove(&self.state, wt, &id.to_string());
            if run.is_none() {
                let _ = tmux.execute(&["kill-window", "-t", &window]);
            } else {
                for key in [
                    "@ws_slot",
                    "@ws_agent",
                    "@ws_state",
                    "@ws_session",
                    "@ws_pane",
                    "automatic-rename",
                ] {
                    let _ = tmux.unset_window_option(&window, key);
                }
            }
            let _ = self.transition(
                &id.to_string(),
                ws_core::state::SessionState::Failed,
                parent_session_id,
            );
        }
        result
    }
}

/// An agent launch recorded under the store lock. Step 2 runs tmux without the lock.
pub(crate) struct AgentLaunch {
    place: Place,
    agent: String,
    parent_session_id: Option<SessionId>,
    id: Ulid,
    slot: u64,
    session_name: String,
    existed: bool,
    run_pane: Option<String>,
    was_running: bool,
    pending: crate::agents::launch::PendingLaunch,
    args: Vec<String>,
}

impl AgentLaunch {
    /// Step 2: opens the tmux window. Callers may run this without the store lock.
    fn launch(&self) -> Result<String> {
        let tmux = Tmux::from_env();
        Ok(match &self.run_pane {
            Some(pane) => tmux.window_for_pane(pane)?,
            None => tmux.execute(&self.args.iter().map(String::as_str).collect::<Vec<_>>())?,
        })
    }
}

/// Step 2 of a removal: runs git without the store lock.
fn remove_files(plan: &RemovePlan) -> Result<Removed> {
    let RemovePlan {
        request,
        wt,
        repo,
        branch,
        ..
    } = plan;
    let wt = wt.as_path();
    if !request.force && !git(wt, &["status", "--porcelain"])?.is_empty() {
        return Err(failure(format!(
            "workspace has uncommitted changes: {} (use --force)",
            wt.display()
        )));
    }
    let tmux = Tmux::from_env();
    // Capture ids before removing the directory. Kill tabs after durable state changes.
    let session_ids = tmux
        .execute(&["list-sessions", "-F", "#{session_id}\t#{@ws_path}"])
        .unwrap_or_default();
    let window_ids = tmux
        .execute(&["list-windows", "-a", "-F", "#{window_id}\t#{@ws_path}"])
        .unwrap_or_default();
    let same_path = |path: &str| Path::new(path).canonicalize().ok() == wt.canonicalize().ok();
    let targets = |rows: &str| {
        rows.lines()
            .filter_map(|line| line.split_once('\t'))
            .filter(|(_, path)| same_path(path))
            .map(|(id, _)| id.to_owned())
            .collect::<Vec<_>>()
    };
    let session_ids = targets(&session_ids);
    let window_ids = targets(&window_ids);
    let mut args = vec!["worktree", "remove"];
    if request.force {
        args.extend(["--force", "--force"]);
    }
    args.push(wt.to_str().unwrap());
    git(repo, &args)
        .map_err(|_| failure(format!("git worktree remove failed: {}", wt.display())))?;
    let _ = fs::remove_dir(wt.parent().unwrap());
    let mut warning = None;
    let mut git_stderr = String::new();
    if !branch.is_empty() && (request.delete_branch || request.force_delete_branch) {
        let flag = if request.force_delete_branch {
            "-D"
        } else {
            "-d"
        };
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["branch", flag, branch])
            .output()?;
        git_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() && !request.force_delete_branch {
            warning = Some(format!(
                "kept unmerged branch {branch} (git branch -D to force)"
            ));
        }
    }
    Ok(Removed {
        session_ids,
        window_ids,
        warning,
        git_stderr,
    })
}

struct RemovePlan {
    request: RemoveWorkspace,
    name: String,
    wt: PathBuf,
    repo: PathBuf,
    branch: String,
    _reservation: Reservation,
}

struct Removed {
    session_ids: Vec<String>,
    window_ids: Vec<String>,
    warning: Option<String>,
    git_stderr: String,
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

fn live_windows(workspace: &Workspace) -> Vec<Value> {
    let rows = Tmux::from_env()
        .execute(&[
            "list-windows",
            "-a",
            "-F",
            "#{window_id}\t#{@ws_session}\t#{window_active}",
        ])
        .unwrap_or_default();
    workspace.body["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|window| {
            let fields = rows
                .lines()
                .map(|line| line.split('\t').collect::<Vec<_>>())
                .find(|fields| {
                    fields.len() == 3
                        && Some(fields[0]) == window["window_id"].as_str()
                        && Some(fields[1]) == window["session_id"].as_str()
                })?;
            let mut window = window.clone();
            window["active"] = json!(fields[2] == "1");
            Some(window)
        })
        .collect()
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
