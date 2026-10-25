//! Local phase launches and session-owned review worktrees.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use ws_core::ids::SessionId;

use crate::{
    api::{ApiError, SessionResponse, Store},
    loops::LoopChild,
    sessions::{Error, Result, Sessions},
    supervisor::{self, CreateHeadless},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnPhase {
    pub parent_session_id: SessionId,
    pub phase: Phase,
    pub repo: PathBuf,
    pub base: String,
    pub caps_required: Vec<String>,
    /// Maximum runtime in seconds.
    pub max_runtime: u64,
    /// Makes the loop session the parent of the phase. The caller becomes the actor.
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_child: Option<LoopChild>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseWorktree {
    pub session: String,
    pub repo: PathBuf,
    pub reference: String,
    pub parent_session_id: Option<SessionId>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/phases/spawn", post(spawn))
        .route("/v1/phases/worktree", post(worktree))
}

async fn spawn(
    State(store): State<Store>,
    Json(request): Json<SpawnPhase>,
) -> std::result::Result<(StatusCode, Json<SessionResponse>), ApiError> {
    if request.base.trim().is_empty() || request.max_runtime == 0 {
        return Err(ApiError(Error::InvalidRequest(
            "Specify a base and a positive maximum runtime.",
        )));
    }
    let metadata = serde_json::to_value(&request).map_err(|error| ApiError(Error::Json(error)))?;
    let session = supervisor::launch(
        store.clone(),
        CreateHeadless {
            headless: true,
            command: request.phase.command,
            cwd: request.repo,
            parent_session_id: Some(request.parent_session_id),
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
            stderr: None,
            key: None,
            loop_child: request.loop_child.map(Box::new),
        },
        metadata,
    )
    .await?;
    let id = session.id;
    let repo = serde_json::json!({"path": session.cwd, "base": request.base});
    let session = crate::api::call(store, move |sessions| {
        sessions.append(id, "session.repo_attached", repo)?;
        // Keep the launch snapshot when the process exits before this append.
        let mut session = session;
        session.repos = sessions.get_record(&id.to_string())?.repos;
        Ok(session)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(session.into())))
}

async fn worktree(
    State(store): State<Store>,
    Json(request): Json<PhaseWorktree>,
) -> std::result::Result<(StatusCode, Json<PathBuf>), ApiError> {
    let path = crate::api::call(store, move |sessions| sessions.phase_worktree(request)).await?;
    Ok((StatusCode::CREATED, Json(path)))
}

impl Sessions {
    pub fn phase_worktree(&mut self, request: PhaseWorktree) -> Result<PathBuf> {
        self.record_call(
            request.parent_session_id,
            "phase.worktree",
            Some(&request.session),
        )?;
        let session = self.get_record(&request.session)?;
        if ["finished", "failed"].contains(&session.state.as_str()) {
            return Err(Error::InvalidRequest("The session has finished or failed."));
        }
        if request.reference.trim().is_empty() {
            return Err(Error::InvalidRequest("Specify a git reference."));
        }
        let repo = request.repo.canonicalize()?;
        git(&repo, &["rev-parse", "--git-dir"])?;
        let root = self
            .state
            .canonicalize()?
            .parent()
            .ok_or(Error::InvalidRequest("The state directory has no parent."))?
            .join("phases");
        fs::create_dir_all(&root)?;
        let path = root.join(session.id.to_string());
        if path.try_exists()? {
            return Err(Error::InvalidRequest(
                "The session already has a phase worktree.",
            ));
        }
        // Retire an interrupted checkout before its path can belong to another repository.
        self.cleanup_phase_worktree(session.id)?;
        // Record the intent first so a restart can collect an interrupted creation.
        self.append(
            session.id,
            "phase.worktree",
            serde_json::json!({
                "repo": repo, "path": path, "reference": request.reference,
            }),
        )?;
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--detach",
                "--",
                path_text(&path)?,
                &request.reference,
            ],
        )?;
        Ok(path)
    }

    pub(crate) fn cleanup_phase_worktree(&mut self, id: SessionId) -> Result<()> {
        let Some(entry) = self.registry.latest_event(id, "phase.worktree")? else {
            return Ok(());
        };
        if self
            .registry
            .latest_event(id, "phase.worktree.removed")?
            .is_some_and(|removed| removed.seq > entry.seq)
        {
            return Ok(());
        }
        let repo = PathBuf::from(entry.body["repo"].as_str().ok_or(Error::InvalidRequest(
            "The phase worktree repository is missing.",
        ))?);
        let path = PathBuf::from(
            entry.body["path"]
                .as_str()
                .ok_or(Error::InvalidRequest("The phase worktree path is missing."))?,
        );
        if path.try_exists()? {
            git(
                &repo,
                &["worktree", "remove", "--force", "--", path_text(&path)?],
            )?;
        }
        git(&repo, &["worktree", "prune"])?;
        self.append(id, "phase.worktree.removed", entry.body)
    }

    pub(crate) fn recover_phase_worktrees(&mut self) -> Result<()> {
        for session in self.registry.sessions()? {
            if ["finished", "failed"].contains(&session.state.as_str())
                && let Err(error) = self.cleanup_phase_worktree(session.id)
            {
                eprintln!(
                    "Cannot remove the phase worktree for {}: {error}",
                    session.id
                );
            }
        }
        Ok(())
    }
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or(Error::InvalidRequest("The git path is not UTF-8."))
}

fn git(repo: &Path, arguments: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    struct ProcessGroup(u32);

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            // SAFETY: The test child leads this sandbox process group.
            unsafe {
                libc::kill(-(self.0 as i32), libc::SIGKILL);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn runtime_limit_sends_term_then_kill_to_the_whole_process_group() {
        let sandbox = tempfile::tempdir_in("/tmp").unwrap();
        let app = crate::api::router(crate::sessions::Sessions::open(sandbox.path(), 1).unwrap());
        let (_, parent) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let script = "/bin/sh -c 'trap \"\" TERM; printf \"%s\" \"$$\" > descendant; exec /bin/sleep 3600' & trap 'printf term > term' TERM; printf ready > ready; while :; do :; done";
        let (status, child) = request(
            &app,
            "POST",
            "/v1/phases/spawn",
            json!({
                "parent_session_id": parent["id"], "phase": {"command": ["/bin/sh", "-c", script]},
                "repo": sandbox.path(), "base": "main", "caps_required": [], "max_runtime": 3600,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let id = child["id"].as_str().unwrap();
        let log = ws_core::log::SessionLog::new(
            sandbox.path(),
            id.parse().unwrap(),
            ws_core::ids::Ulid::new(),
        );
        let entries = log.read().unwrap().entries;
        let pid = entries
            .iter()
            .find(|entry| entry.event_type == "session.running")
            .unwrap()
            .body["process"]["pid"]
            .as_u64()
            .unwrap() as u32;
        let _group = ProcessGroup(pid);
        wait_for("the ready event", false, || {
            sandbox.path().join("ready").exists()
                && fs::read_to_string(sandbox.path().join("descendant"))
                    .is_ok_and(|text| text.parse::<u32>().is_ok())
        })
        .await;
        let descendant = fs::read_to_string(sandbox.path().join("descendant")).unwrap();
        tokio::time::advance(Duration::from_secs(3600)).await;
        wait_for("the TERM event", false, || {
            fs::read_to_string(sandbox.path().join("term")).is_ok_and(|text| text == "term")
        })
        .await;
        assert_eq!(
            fs::read_to_string(sandbox.path().join("term")).unwrap(),
            "term"
        );
        tokio::time::advance(Duration::from_secs(10)).await;
        wait_for("the failed state", true, || {
            log.read()
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.event_type == "session.failed")
        })
        .await;
        let (_, child) = request(&app, "GET", &format!("/v1/sessions/{id}"), Value::Null).await;
        assert_eq!(child["state"], "failed");
        assert_eq!(child["exit_code"], 137);
        assert!(
            log.read()
                .unwrap()
                .entries
                .iter()
                .any(|entry| entry.event_type == "session.failed"
                    && entry.body["reason"] == "The phase exceeded its maximum runtime.")
        );
        wait_for("the descendant exit", true, || {
            let output = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &descendant])
                .output()
                .unwrap();
            !output.status.success()
                || String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .starts_with('Z')
        })
        .await;
    }

    #[tokio::test]
    async fn a_phase_with_a_loop_object_is_a_child_of_the_loop_and_records_the_actor() {
        let sandbox = tempfile::tempdir_in("/tmp").unwrap();
        let app = crate::api::router(crate::sessions::Sessions::open(sandbox.path(), 1).unwrap());
        let directory = sandbox.path().join("loop");
        fs::create_dir(&directory).unwrap();
        let (_, loop_session) = request(
            &app,
            "POST",
            "/v1/loops/create",
            json!({"loop_dir": directory, "parent_session_id": null}),
        )
        .await;
        let (_, caller) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let script = format!(
            "printf '%s|%s|%s' \"$WS_LOOP_ID\" \"$WS_LOOP_PHASE\" \"$WS_LOOP_ATTEMPT\" > '{}'",
            sandbox.path().join("environment").display()
        );
        let (status, child) = request(
            &app,
            "POST",
            "/v1/phases/spawn",
            json!({
                "parent_session_id": caller["id"], "phase": {"command": ["/bin/sh", "-c", script]},
                "repo": sandbox.path(), "base": "main", "caps_required": [], "max_runtime": 3600,
                "loop": {"loop": loop_session["id"], "phase": 110, "attempt": 3},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(child["parent_session_id"], loop_session["id"]);
        let id = child["id"].as_str().unwrap().to_owned();
        let log = ws_core::log::SessionLog::new(
            sandbox.path(),
            id.parse().unwrap(),
            ws_core::ids::Ulid::new(),
        );
        let created = log
            .read()
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.event_type == "session.created")
            .unwrap();
        assert_eq!(created.body["actor"], caller["id"]);
        assert_eq!(created.body["parent_session_id"], loop_session["id"]);
        let environment = sandbox.path().join("environment");
        wait_for("the environment file", false, || {
            fs::read_to_string(&environment).is_ok_and(|text| text.ends_with("|3"))
        })
        .await;
        assert_eq!(
            fs::read_to_string(&environment).unwrap(),
            format!("{}|110|3", loop_session["id"].as_str().unwrap())
        );
    }

    async fn wait_for(event: &str, advance_clock: bool, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "The process did not produce {event}."
            );
            if advance_clock {
                // After escalation, advance the timers that poll process group membership.
                tokio::time::advance(Duration::from_millis(20)).await;
            } else {
                // Keep the deadline stationary until the external process writes its signal.
                tokio::task::yield_now().await;
            }
        }
    }

    async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}
