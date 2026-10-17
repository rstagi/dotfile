//! Local phase launches through the headless session supervisor.

use std::{collections::BTreeMap, path::PathBuf};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};
use ws_core::ids::SessionId;

use crate::{
    api::{ApiError, SessionResponse, Store},
    sessions::Error,
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
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub command: Vec<String>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/phases/spawn", post(spawn))
}

async fn spawn(
    State(store): State<Store>,
    Json(request): Json<SpawnPhase>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    if request.base.trim().is_empty() || request.max_runtime == 0 {
        return Err(ApiError(Error::InvalidRequest(
            "Specify a base and a positive maximum runtime.",
        )));
    }
    let metadata = serde_json::to_value(&request).map_err(|error| ApiError(Error::Json(error)))?;
    let session = supervisor::launch(
        store,
        CreateHeadless {
            headless: true,
            command: request.phase.command,
            cwd: request.repo,
            parent_session_id: Some(request.parent_session_id),
            env: BTreeMap::new(),
            stdin: None,
            stdout: None,
            stderr: None,
        },
        metadata,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(session.into())))
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
        wait_for(|| {
            sandbox.path().join("ready").exists()
                && fs::read_to_string(sandbox.path().join("descendant"))
                    .is_ok_and(|text| text.parse::<u32>().is_ok())
        })
        .await;
        let descendant = fs::read_to_string(sandbox.path().join("descendant")).unwrap();
        tokio::time::advance(Duration::from_secs(3600)).await;
        wait_for(|| {
            fs::read_to_string(sandbox.path().join("term")).is_ok_and(|text| text == "term")
        })
        .await;
        assert_eq!(
            fs::read_to_string(sandbox.path().join("term")).unwrap(),
            "term"
        );
        tokio::time::advance(Duration::from_secs(10)).await;
        wait_for(|| {
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
        wait_for(|| {
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

    async fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "The expected process event did not occur."
            );
            // Keep the paused clock stationary until the external process writes the event.
            tokio::task::yield_now().await;
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
