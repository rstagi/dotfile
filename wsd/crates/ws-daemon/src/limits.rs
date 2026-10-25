//! Session spawn limits, checked while the lifecycle store is locked.

use crate::sessions::{Error, Result, Sessions};
use std::{
    collections::{HashMap, HashSet},
    fmt,
};
use ws_core::ids::SessionId;

#[derive(Debug)]
pub enum SpawnLimit {
    Children { parent: SessionId, limit: usize },
    Depth { parent: SessionId, limit: usize },
    Spawns { parent: SessionId, limit: usize },
}

impl Sessions {
    pub(crate) fn check_spawn_limits(&self, parent: Option<SessionId>) -> Result<()> {
        let Some(parent) = parent else {
            return Ok(());
        };
        self.get_record(&parent.to_string())?;
        let scope = self.spawn_scope(Some(parent))?;
        if let Some(scope) = &scope {
            self.check_loop_spawn_cap(parent, scope)?;
        }
        let sessions = self.registry.sessions()?;
        let limit = self.config.limits.max_children;
        let parent_id = parent.to_string();
        let count = sessions
            .iter()
            .filter(|session| session.body["parent_session_id"].as_str() == Some(&parent_id))
            .filter(|session| {
                scope.is_none() || !["finished", "failed"].contains(&session.state.as_str())
            })
            .count();
        if count >= limit {
            return Err(Error::SpawnLimit(SpawnLimit::Children { parent, limit }));
        }
        let limit = self.config.limits.max_depth;
        let mut ancestor = Some(parent);
        let mut depth = 0;
        let mut visited = HashSet::new();
        while let Some(id) = ancestor {
            if depth >= limit {
                return Err(Error::SpawnLimit(SpawnLimit::Depth { parent, limit }));
            }
            if !visited.insert(id) {
                return Err(Error::InvalidRequest(
                    "The session ancestry contains a cycle.",
                ));
            }
            let session = sessions
                .iter()
                .find(|session| session.id == id)
                .ok_or_else(|| ws_core::registry::Error::NotFound(id.to_string()))?;
            ancestor = session.body["parent_session_id"]
                .as_str()
                .map(str::parse::<SessionId>)
                .transpose()
                .map_err(|_| {
                    Error::InvalidRequest("The session ancestry contains an invalid id.")
                })?;
            depth += 1;
        }
        Ok(())
    }

    pub(crate) fn spawn_scope(&self, mut parent: Option<SessionId>) -> Result<Option<String>> {
        let mut visited = HashSet::new();
        while let Some(id) = parent {
            if !visited.insert(id) {
                return Err(Error::InvalidRequest(
                    "The session ancestry contains a cycle.",
                ));
            }
            let session = self.get_record(&id.to_string())?;
            if session.body["kind"] == "loop" {
                return Ok(Some(format!("session:{id}")));
            }
            if let Some(scope) = legacy_loop_scope(&session.body) {
                return Ok(Some(scope));
            }
            parent = session.body["parent_session_id"]
                .as_str()
                .map(str::parse::<SessionId>)
                .transpose()
                .map_err(|_| {
                    Error::InvalidRequest("The session ancestry contains an invalid id.")
                })?;
        }
        Ok(None)
    }

    fn check_loop_spawn_cap(&self, parent: SessionId, scope: &str) -> Result<()> {
        let creations = self.registry.events_of_type("session.created")?;
        let bodies: HashMap<_, _> = creations
            .iter()
            .map(|entry| (entry.session_id, &entry.body))
            .collect();
        let mut count = 0;
        for entry in &creations {
            let recorded = entry.body["loop_spawn_scope"].as_str().map(str::to_owned);
            let inferred = || {
                legacy_loop_scope(&entry.body).or_else(|| {
                    let mut ancestor = entry.body["parent_session_id"]
                        .as_str()?
                        .parse::<SessionId>()
                        .ok();
                    let mut seen = HashSet::new();
                    while let Some(id) = ancestor.filter(|id| seen.insert(*id)) {
                        let body = bodies.get(&id)?;
                        if body["kind"] == "loop" {
                            return Some(format!("session:{id}"));
                        }
                        if let Some(scope) = legacy_loop_scope(body) {
                            return Some(scope);
                        }
                        ancestor = body["parent_session_id"].as_str()?.parse().ok();
                    }
                    None
                })
            };
            if recorded.or_else(inferred).as_deref() == Some(scope) {
                count += 1;
            }
        }
        count += self
            .registry
            .events_of_type("session.reparented")?
            .iter()
            .filter(|entry| entry.body["loop_spawn_scope"].as_str() == Some(scope))
            .count();
        let limit = self.config.r#loop.max_spawns;
        if count >= limit {
            return Err(Error::SpawnLimit(SpawnLimit::Spawns { parent, limit }));
        }
        Ok(())
    }

    pub(crate) fn tag_loop_spawn(
        &self,
        id: SessionId,
        event_type: &str,
        body: &mut serde_json::Value,
    ) -> Result<()> {
        let field = match event_type {
            "session.created" => "parent_session_id",
            "session.reparented" => "parent",
            _ => return Ok(()),
        };
        let parent = body[field]
            .as_str()
            .map(str::parse::<SessionId>)
            .transpose()
            .map_err(|_| Error::InvalidRequest("The parent session id is invalid."))?;
        if let Some(scope) = legacy_loop_scope(body).or(self.spawn_scope(parent)?) {
            self.check_loop_spawn_cap(parent.unwrap_or(id), &scope)?;
            body["loop_spawn_scope"] = serde_json::json!(scope);
        }
        Ok(())
    }
}

fn legacy_loop_scope(body: &serde_json::Value) -> Option<String> {
    if let Some(run_id) = body["run_id"].as_str().filter(|id| !id.is_empty()) {
        return Some(format!("run:{run_id}"));
    }
    let command = body["command"].as_array()?;
    let program = std::path::Path::new(command.first()?.as_str()?)
        .file_name()?
        .to_str()?;
    let script = if program == "loop-orchestrator.sh" {
        0
    } else if ["zsh", "bash", "sh"].contains(&program)
        && std::path::Path::new(command.get(1)?.as_str()?).file_name()? == "loop-orchestrator.sh"
    {
        1
    } else {
        return None;
    };
    command[script + 1..].windows(2).find_map(|args| {
        (args[0] == "--run-id")
            .then(|| args[1].as_str())
            .flatten()
            .filter(|id| !id.is_empty())
            .map(|id| format!("run:{id}"))
    })
}

impl fmt::Display for SpawnLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawns { parent, limit } => write!(
                formatter,
                "The loop of session {parent} reached its spawn limit of {limit}."
            ),
            Self::Depth { parent, limit } => write!(
                formatter,
                "A child of session {parent} exceeds the depth limit of {limit}."
            ),
            Self::Children { parent, limit } => write!(
                formatter,
                "Session {parent} reached its child limit of {limit}."
            ),
        }
    }
}

impl std::error::Error for SpawnLimit {}

#[cfg(test)]
mod tests {
    use crate::{api, sessions::Sessions};
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use ws_core::ids::SessionId;

    fn loop_session(sessions: &mut Sessions) -> SessionId {
        let id = SessionId::new();
        let alias = sessions.next_alias().unwrap();
        sessions
            .append(id, "session.created", json!({"alias":alias,"kind":"loop"}))
            .unwrap();
        id
    }

    async fn post(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn loop_spawn_cap_counts_finished_descendants_after_restart() {
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("config.toml"),
            "[loop]\nmax_spawns = 2\n",
        )
        .unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let root = loop_session(&mut sessions);
        let app = api::router(sessions);
        let (status, child) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, grandchild) = post(
            &app,
            "/v1/sessions",
            json!({"parent_session_id":child["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        for session in [&grandchild, &child] {
            assert_eq!(
                post(
                    &app,
                    &format!("/v1/sessions/{}/done", session["id"].as_str().unwrap()),
                    json!({})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        drop(app);
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, error) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_spawns_exceeded");
        assert_eq!(error["error"]["limit"], 2);
    }

    #[tokio::test]
    async fn reparent_rejects_a_full_loop_and_counts_moves_after_detach() {
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("config.toml"),
            "[limits]\nmax_children = 1\n[loop]\nmax_spawns = 2\n",
        )
        .unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let root = loop_session(&mut sessions);
        let app = api::router(sessions);
        let (_, child) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        let (_, other) = post(&app, "/v1/sessions", json!({})).await;
        let path = format!("/v1/sessions/{}/parent", other["id"].as_str().unwrap());
        let (status, error) = post(&app, &path, json!({"parent":root.to_string()})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_children_exceeded");
        post(
            &app,
            &format!("/v1/sessions/{}/done", child["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        assert_eq!(
            post(&app, &path, json!({"parent":root.to_string()}))
                .await
                .0,
            StatusCode::OK
        );
        // Repeating the same parent consumes no additional spawn.
        assert_eq!(
            post(&app, &path, json!({"parent":root.to_string()}))
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            post(&app, &path, json!({"parent":null})).await.0,
            StatusCode::OK
        );
        drop(app);
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, error) = post(&app, &path, json!({"parent":root.to_string()})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_spawns_exceeded");
    }

    #[tokio::test]
    async fn old_engine_allows_ten_subs_and_ten_attempts_per_sub() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let supervise = SessionId::new();
        let alias = sessions.next_alias().unwrap();
        sessions
            .append(
                supervise,
                "session.created",
                json!({"alias":alias,"kind":"headless",
            "command":["zsh","/runtime/loop-orchestrator.sh","--run-id","old-run"]}),
            )
            .unwrap();
        let app = api::router(sessions);
        for _ in 0..10 {
            let (status, sub) =
                post(&app, "/v1/sessions", json!({"parent_session_id":supervise})).await;
            assert_eq!(status, StatusCode::CREATED, "{sub}");
            for _ in 0..10 {
                let (status, attempt) =
                    post(&app, "/v1/sessions", json!({"parent_session_id":sub["id"]})).await;
                assert_eq!(status, StatusCode::CREATED, "{attempt}");
                assert_eq!(
                    post(
                        &app,
                        &format!("/v1/sessions/{}/done", attempt["id"].as_str().unwrap()),
                        json!({})
                    )
                    .await
                    .0,
                    StatusCode::OK
                );
            }
            assert_eq!(
                post(
                    &app,
                    &format!("/v1/sessions/{}/done", sub["id"].as_str().unwrap()),
                    json!({})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
    }

    #[tokio::test]
    async fn old_engine_spawn_cap_uses_the_run_id_across_roots_and_restarts() {
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("config.toml"),
            "[loop]\nmax_spawns = 2\n",
        )
        .unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let supervise = SessionId::new();
        let alias = sessions.next_alias().unwrap();
        sessions
            .append(
                supervise,
                "session.created",
                json!({"alias":alias,"kind":"headless",
            "command":["zsh","/runtime/loop-orchestrator.sh","--run-id","old-run"]}),
            )
            .unwrap();
        let app = api::router(sessions);
        let (status, child) =
            post(&app, "/v1/sessions", json!({"parent_session_id":supervise})).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            post(
                &app,
                &format!("/v1/sessions/{}/done", child["id"].as_str().unwrap()),
                json!({})
            )
            .await
            .0,
            StatusCode::OK
        );
        drop(app);
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let script = sandbox.path().join("loop-orchestrator.sh");
        let marker = sandbox.path().join("launched");
        std::fs::write(&script, format!("touch '{}'\n", marker.display())).unwrap();
        let (status, error) = post(
            &app,
            "/v1/sessions",
            json!({"headless":true,
            "cwd":sandbox.path(),"command":["/bin/sh",script,"--run-id","old-run"]}),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_spawns_exceeded");
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn default_live_child_limit_still_applies_to_loops() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let root = loop_session(&mut sessions);
        let app = api::router(sessions);
        let mut children = Vec::new();
        for _ in 0..8 {
            let (status, child) =
                post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
            assert_eq!(status, StatusCode::CREATED);
            children.push(child);
        }
        let (status, error) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error["error"]["code"], "max_children_exceeded");
        assert_eq!(
            post(
                &app,
                &format!("/v1/sessions/{}/done", children[0]["id"].as_str().unwrap()),
                json!({})
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            post(&app, "/v1/sessions", json!({"parent_session_id":root}))
                .await
                .0,
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn default_non_loop_limit_still_counts_finished_children() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, root) = post(&app, "/v1/sessions", json!({})).await;
        for _ in 0..8 {
            let (status, child) = post(
                &app,
                "/v1/sessions",
                json!({"parent_session_id":root["id"]}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(
                post(
                    &app,
                    &format!("/v1/sessions/{}/done", child["id"].as_str().unwrap()),
                    json!({})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        let (status, error) = post(
            &app,
            "/v1/sessions",
            json!({"parent_session_id":root["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error["error"]["code"], "max_children_exceeded");
    }

    #[tokio::test]
    async fn spawn_cap_counts_sessions_created_before_scope_tags() {
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("config.toml"),
            "[loop]\nmax_spawns = 1\n",
        )
        .unwrap();
        let device = ws_core::device::Device::init(sandbox.path(), "test").unwrap();
        let root = SessionId::new();
        ws_core::log::SessionLog::new(sandbox.path(), root, device.device_id)
            .append(0, "session.created", json!({"alias":1,"kind":"loop"}))
            .unwrap();
        let child = SessionId::new();
        let log = ws_core::log::SessionLog::new(sandbox.path(), child, device.device_id);
        log.append(
            0,
            "session.created",
            json!({"alias":2,"parent_session_id":root}),
        )
        .unwrap();
        log.append(0, "session.finished", json!({})).unwrap();
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, error) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_spawns_exceeded");
    }

    #[tokio::test]
    async fn ordinary_command_arguments_do_not_enable_loop_limits() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let root = SessionId::new();
        let alias = sessions.next_alias().unwrap();
        sessions
            .append(
                root,
                "session.created",
                json!({"alias":alias,"kind":"headless",
            "command":["/bin/echo","/runtime/loop-orchestrator.sh","--run-id","documentation"]}),
            )
            .unwrap();
        let app = api::router(sessions);
        for _ in 0..8 {
            let (status, child) =
                post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(
                post(
                    &app,
                    &format!("/v1/sessions/{}/done", child["id"].as_str().unwrap()),
                    json!({})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        let (status, error) = post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
        assert_eq!(error["error"]["code"], "max_children_exceeded");
    }

    #[tokio::test]
    async fn loop_allows_ten_sequential_reviews_with_default_limits() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let root = loop_session(&mut sessions);
        let app = api::router(sessions);
        for _ in 0..10 {
            let (status, child) =
                post(&app, "/v1/sessions", json!({"parent_session_id":root})).await;
            assert_eq!(status, StatusCode::CREATED, "{child}");
            let (status, result) = post(
                &app,
                &format!("/v1/sessions/{}/done", child["id"].as_str().unwrap()),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{result}");
        }
    }
}
