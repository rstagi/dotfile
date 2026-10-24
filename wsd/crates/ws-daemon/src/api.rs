//! Versioned session lifecycle routes.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ws_core::{ids::SessionId, registry::Session};

use crate::sessions::{CreateSession, Error, Sessions};

pub(crate) type Store = Arc<Mutex<Sessions>>;

#[derive(Serialize)]
pub(crate) struct SessionResponse {
    #[serde(flatten)]
    session: Session,
    parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CreateRequest {
    Headless(crate::supervisor::CreateHeadless),
    Session(CreateSession),
}

pub(crate) struct ApiError(pub(crate) Error);

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallContext {
    pub(crate) parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReparentSession {
    parent: Option<String>,
    parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameSession {
    name: String,
    parent_session_id: Option<SessionId>,
}

/// Builds routes over one serialized lifecycle service. File operations use blocking workers.
/// POST bodies and GET query parameters accept the optional caller field `parent_session_id`.
pub fn router(sessions: Sessions) -> Router {
    let store = Arc::new(Mutex::new(sessions));
    router_with_store(store)
}

pub(crate) fn router_with_store(store: Store) -> Router {
    Router::new()
        .merge(crate::import::backend::router())
        .merge(crate::bus::router())
        .merge(crate::edges::router())
        .merge(crate::triggers::router())
        .merge(crate::wait::router())
        .merge(crate::hooks::router())
        .merge(crate::notify::router())
        .merge(crate::workspaces::router())
        .merge(crate::agents::router())
        .merge(crate::phases::router())
        .merge(crate::loops::router())
        .merge(crate::tail::router())
        .merge(crate::quota::usage::router())
        .route("/v1/sessions", get(list).post(create))
        .route(
            "/v1/sessions/start",
            axum::routing::post(crate::places::start),
        )
        .route("/v1/sessions/{id}", get(fetch))
        .route("/v1/sessions/{id}/stop", axum::routing::post(stop))
        .route("/v1/sessions/{id}/name", axum::routing::post(rename))
        .route("/v1/sessions/{id}/parent", axum::routing::post(reparent))
        .with_state(store)
}

async fn create(
    State(store): State<Store>,
    Json(request): Json<CreateRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    let session = match request {
        CreateRequest::Headless(request) => crate::supervisor::create(store, request).await?,
        CreateRequest::Session(request) => {
            call(store, move |sessions| sessions.create(request)).await?
        }
    };
    Ok((StatusCode::CREATED, Json(session.into())))
}

async fn list(
    State(store): State<Store>,
    Query(context): Query<CallContext>,
) -> Result<Json<Vec<SessionResponse>>, ApiError> {
    let sessions = call(store, move |sessions| {
        sessions.list(context.parent_session_id)
    })
    .await?;
    Ok(Json(sessions.into_iter().map(Into::into).collect()))
}

async fn fetch(
    State(store): State<Store>,
    Path(id): Path<String>,
    Query(context): Query<CallContext>,
) -> Result<Json<SessionResponse>, ApiError> {
    Ok(Json(
        call(store, move |sessions| {
            sessions.get(&id, context.parent_session_id)
        })
        .await?
        .into(),
    ))
}

async fn rename(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<RenameSession>,
) -> Result<Json<SessionResponse>, ApiError> {
    Ok(Json(
        call(store, move |sessions| {
            sessions.rename(&id, &request.name, request.parent_session_id)
        })
        .await?
        .into(),
    ))
}

async fn reparent(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<ReparentSession>,
) -> Result<Json<SessionResponse>, ApiError> {
    Ok(Json(
        call(store, move |sessions| {
            sessions.reparent(&id, request.parent.as_deref(), request.parent_session_id)
        })
        .await?
        .into(),
    ))
}

async fn stop(
    State(store): State<Store>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<SessionResponse>, ApiError> {
    let context = if body.is_empty() {
        CallContext::default()
    } else {
        serde_json::from_slice::<CallContext>(&body).map_err(|_| {
            ApiError(Error::InvalidRequest(
                "The stop request must contain a valid caller context.",
            ))
        })?
    };
    Ok(Json(
        crate::supervisor::stop(store, id, context.parent_session_id)
            .await?
            .into(),
    ))
}

pub(crate) async fn call<T: Send + 'static>(
    store: Store,
    operation: impl FnOnce(&mut Sessions) -> crate::sessions::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut sessions = store
            .lock()
            .map_err(|_| Error::Io(std::io::Error::other("Cannot lock the session store.")))?;
        let result = operation(&mut sessions);
        // Exit producers retire the old launch before actions can reopen its session.
        if std::mem::take(&mut sessions.trigger_events_pending)
            && let Err(error) = sessions.evaluate_triggers()
        {
            eprintln!("Cannot evaluate triggers: {error}");
        }
        result
    })
    .await
    .map_err(|_| {
        ApiError(Error::Io(std::io::Error::other(
            "The session worker failed.",
        )))
    })?
    .map_err(ApiError)
}

impl From<Session> for SessionResponse {
    fn from(session: Session) -> Self {
        let parent_session_id = session
            .body
            .get("parent_session_id")
            .and_then(|value| value.as_str())
            .and_then(|id| id.parse().ok());
        Self {
            session,
            parent_session_id,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        use ws_core::registry;
        let (status, code) = match &self.0 {
            Error::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            Error::Registry(registry::Error::NotFound(_)) => {
                (StatusCode::NOT_FOUND, "session_not_found")
            }
            Error::Registry(registry::Error::Ambiguous(_)) => {
                (StatusCode::CONFLICT, "ambiguous_session")
            }
            Error::ShuttingDown => (StatusCode::SERVICE_UNAVAILABLE, "daemon_stopping"),
            Error::WaitTimeout => (StatusCode::REQUEST_TIMEOUT, "wait_timeout"),
            Error::SpawnLimit(crate::limits::SpawnLimit::Children { .. }) => {
                (StatusCode::TOO_MANY_REQUESTS, "max_children_exceeded")
            }
            Error::SpawnLimit(crate::limits::SpawnLimit::Depth { .. }) => {
                (StatusCode::TOO_MANY_REQUESTS, "max_depth_exceeded")
            }
            Error::PendingMessages => (StatusCode::CONFLICT, "pending_messages"),
            Error::DependencyCycle(_) => (StatusCode::CONFLICT, "dependency_cycle"),
            Error::NameTaken(_) => (StatusCode::CONFLICT, "name_taken"),
            Error::Transition(_) => (StatusCode::CONFLICT, "invalid_transition"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
        };
        let mut error = json!({"code": code, "message": self.0.to_string()});
        if let Error::SpawnLimit(
            crate::limits::SpawnLimit::Children { parent, limit }
            | crate::limits::SpawnLimit::Depth { parent, limit },
        ) = &self.0
        {
            error["parent_session_id"] = json!(parent);
            error["limit"] = json!(limit);
        }
        if let Error::Transition(transition) = self.0 {
            error["from"] = json!(transition.from);
            error["to"] = json!(transition.to);
        }
        (status, Json(json!({"error": error}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::sessions::Sessions;

    #[tokio::test]
    async fn session_end_hooks_append_exit_and_wake_waiters() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let callback = json!({"callback_id":ws_core::ids::Ulid::new(), "session_id":id, "observed_at_us":1, "state":"exit", "pane":null, "tmux_socket":null, "tmux_path":null, "payload":{"hook_event_name":"SessionEnd", "exit_code":7}});
        for code in [json!("invalid"), json!(2147483648_i64)] {
            let mut invalid = callback.clone();
            invalid["callback_id"] = json!(ws_core::ids::Ulid::new());
            invalid["payload"]["exit_code"] = code;
            let (status, _) = request(&app, "POST", "/v1/hooks/ingest", invalid).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        for _ in 0..2 {
            let (status, _) = request(&app, "POST", "/v1/hooks/ingest", callback.clone()).await;
            assert_eq!(status, StatusCode::CREATED);
        }
        let (status, result) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/wait"),
            json!({"for":"exit", "timeout":0}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["event"]["body"]["code"], 7);
        let entries = ws_core::log::SessionLog::new(
            sandbox.path(),
            id.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries;
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.event_type == "exit")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn waits_observe_done_state_and_events_without_blocking_mutations() {
        use std::{future::Future, task::Poll, time::Duration};
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let endpoint = format!("/v1/sessions/{id}/wait");
        let mut waits = Vec::new();
        for condition in ["done", "state=finished", "event=done"] {
            waits.push(Box::pin(request(
                &app,
                "POST",
                &endpoint,
                json!({"for":condition}),
            )));
        }
        for wait in &mut waits {
            std::future::poll_fn(|cx| {
                assert!(wait.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        let (status, _) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/done"),
            json!({"output":"ready"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for wait in waits {
            let (status, result) = tokio::time::timeout(Duration::from_secs(30), wait)
                .await
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!(result["session"]["id"], id);
        }
        let (status, _) =
            request(&app, "POST", &endpoint, json!({"for":"exit", "timeout":0})).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        let (status, _) = request(&app, "POST", &endpoint, json!({"for":"state=invalid"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = request(
            &app,
            "POST",
            "/v1/sessions/missing/wait",
            json!({"for":"done"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        drop(app);
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, result) =
            request(&app, "POST", &endpoint, json!({"for":"done", "timeout":0})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["event"]["type"], "done");
    }

    #[tokio::test]
    async fn done_publishes_output_and_refuses_unread_messages() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let endpoint = format!("/v1/sessions/{id}/done");
        let (status, completed) = request(
            &app,
            "POST",
            &endpoint,
            json!({"output":"alpha \"quoted\" $literal\nbeta"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(completed["state"], "finished");
        let output = sandbox.path().join("sessions").join(id).join("output.md");
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            "alpha \"quoted\" $literal\nbeta"
        );
        let (status, _) = request(&app, "POST", &endpoint, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "");
        let (_, sender) = request(&app, "POST", "/v1/sessions", json!({})).await;
        // A new session needs no agent restart to accept queued work.
        let (_, target) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let target_id = target["id"].as_str().unwrap();
        let (status, _) = request(
            &app,
            "POST",
            "/v1/messages",
            json!({"target":target_id, "body":"pending", "parent_session_id":sender["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, error) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{target_id}/done"),
            json!({"output":"must not publish"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"]["code"], "pending_messages");
        assert!(
            !sandbox
                .path()
                .join("sessions")
                .join(target_id)
                .join("output.md")
                .exists()
        );
        let entries = ws_core::log::SessionLog::new(
            sandbox.path(),
            id.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries;
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.event_type == "done")
                .count(),
            2
        );
        drop(app);
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, rebuilt) = request(&app, "GET", &format!("/v1/sessions/{id}"), Value::Null).await;
        assert_eq!(rebuilt["state"], "finished");
    }

    #[tokio::test]
    async fn dependency_edges_reject_a_named_cycle_and_survive_index_rebuild() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let mut ids = Vec::new();
        for name in ["build", "test", "ship"] {
            let (_, session) = request(&app, "POST", "/v1/sessions", json!({"name": name})).await;
            ids.push(session["id"].as_str().unwrap().to_owned());
        }
        for (source, target, kind) in [(0, 1, "depends_on"), (1, 2, "block_on")] {
            let (status, edge) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{}/{kind}", ids[source]),
                json!({"target": ids[target]}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{edge}");
            assert_eq!(edge["source"], ids[source]);
            assert_eq!(edge["target"], ids[target]);
            assert_eq!(edge["kind"], kind);
        }
        for (source, target) in [(2, 0), (0, 0)] {
            let (status, error) = request(
                &app,
                "POST",
                &format!("/v1/sessions/{}/depends_on", ids[source]),
                json!({"target": ids[target]}),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(error["error"]["code"], "dependency_cycle");
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(&ids[source])
            );
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(&ids[target])
            );
        }
        let (status, edges) = request(&app, "GET", "/v1/edges", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(edges.as_array().unwrap().len(), 2);
        drop(app);
        std::fs::remove_file(sandbox.path().join("registry.sqlite")).unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, rebuilt) = request(&app, "GET", "/v1/edges", Value::Null).await;
        assert_eq!(rebuilt, edges);
        let log = ws_core::log::SessionLog::new(
            sandbox.path(),
            ids[0].parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap();
        assert_eq!(
            log.entries
                .iter()
                .filter(|entry| entry.event_type == "session.depends_on")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_disconnected_hook_request_still_records_its_callback() {
        use std::{future::Future, task::Poll};
        use ws_core::{ids::Ulid, log::SessionLog};

        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, session) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = session["id"].as_str().unwrap();
        let first = Ulid::new();
        let second = Ulid::new();
        let callback = |callback_id, observed_at_us| {
            json!({
                "callback_id": callback_id, "session_id": id,
                "observed_at_us": observed_at_us, "state": "waiting",
                "pane": null, "tmux_socket": null, "tmux_path": null, "payload": {}
            })
        };
        let first_callback = callback(first, 1);
        let guard = crate::hooks::INGEST_LOCK.lock().await;
        let mut disconnected = Box::pin(request(&app, "POST", "/v1/hooks/ingest", first_callback));
        std::future::poll_fn(|cx| {
            assert!(disconnected.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        tokio::task::yield_now().await;
        drop(disconnected);
        drop(guard);
        let (status, _) = request(&app, "POST", "/v1/hooks/ingest", callback(second, 2)).await;
        assert_eq!(status, StatusCode::CREATED);
        let log = SessionLog::new(sandbox.path(), id.parse().unwrap(), Ulid::new());
        let received: Vec<_> = log
            .read()
            .unwrap()
            .entries
            .into_iter()
            .filter(|entry| entry.event_type == "hook.received")
            .map(|entry| entry.body["callback_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(received, [first.to_string(), second.to_string()]);
    }

    #[tokio::test]
    async fn creates_lists_and_gets_sessions_through_the_in_process_client() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 100).unwrap());
        let (status, created) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({"name": "build", "workspace_id": "project"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["alias"], 100);
        assert_eq!(created["name"], "build");
        assert_eq!(created["state"], "created");
        assert_eq!(created["workspace_id"], "project");
        assert_eq!(created["parent_session_id"], Value::Null);
        let id = created["id"].as_str().unwrap();
        let (status, fetched) =
            request(&app, "GET", &format!("/v1/sessions/{id}"), Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched, created);
        let (status, listed) = request(&app, "GET", "/v1/sessions", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed, json!([created]));
    }

    #[tokio::test]
    async fn stop_is_durable_and_a_second_stop_returns_a_typed_conflict() {
        use ws_core::{ids::Ulid, log::SessionLog};

        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, created) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = created["id"].as_str().unwrap();
        let path = format!("/v1/sessions/{id}/stop");
        let (status, stopped) = request(&app, "POST", &path, json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stopped["state"], "finished");
        let (status, error) = request(&app, "POST", &path, json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"]["code"], "invalid_transition");
        assert_eq!(error["error"]["from"], "finished");
        assert_eq!(error["error"]["to"], "finished");
        let log = SessionLog::new(sandbox.path(), id.parse().unwrap(), Ulid::new());
        let entries = log.read().unwrap().entries;
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.event_type.as_str())
                .collect::<Vec<_>>(),
            ["session.created", "session.finished"]
        );
        assert!(entries[1].body["parent_session_id"].is_null());
        drop(app);
        std::fs::remove_file(sandbox.path().join("registry.sqlite")).unwrap();
        let restarted = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, fetched) = request(
            &restarted,
            "GET",
            &format!("/v1/sessions/{id}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched, stopped);
        let (_, next) = request(&restarted, "POST", "/v1/sessions", json!({})).await;
        assert_eq!(next["alias"], 2);
    }

    #[tokio::test]
    async fn records_the_caller_for_create_list_get_stop_and_rejected_calls() {
        use ws_core::{ids::Ulid, log::SessionLog};

        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, parent) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({"name": "orchestrator"}),
        )
        .await;
        let parent = parent["id"].as_str().unwrap();
        let (status, child) = request(
            &app,
            "POST",
            "/v1/sessions",
            json!({"parent_session_id": parent}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(child["parent_session_id"], parent);
        let id = child["id"].as_str().unwrap();
        let (status, listed) = request(
            &app,
            "GET",
            &format!("/v1/sessions?parent_session_id={parent}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 2);
        let (status, _) = request(
            &app,
            "GET",
            &format!("/v1/sessions/{id}?parent_session_id={parent}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/stop"),
            json!({"parent_session_id": parent}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = request(
            &app,
            "GET",
            &format!("/v1/sessions/missing?parent_session_id={parent}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = request(
            &app,
            "POST",
            "/v1/sessions/missing/stop",
            json!({"parent_session_id": parent}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let calls = SessionLog::new(sandbox.path(), parent.parse().unwrap(), Ulid::new())
            .read()
            .unwrap()
            .entries;
        let calls: Vec<_> = calls
            .iter()
            .filter(|entry| entry.event_type == "api.called")
            .collect();
        assert_eq!(
            calls
                .iter()
                .map(|entry| entry.body["operation"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "session.create",
                "session.list",
                "session.get",
                "session.stop",
                "session.get",
                "session.stop"
            ]
        );
        assert!(
            calls
                .iter()
                .all(|entry| entry.body["parent_session_id"] == parent)
        );
        assert_eq!(calls[0].body["target"], id);
        let entries = SessionLog::new(sandbox.path(), id.parse().unwrap(), Ulid::new())
            .read()
            .unwrap()
            .entries;
        assert!(
            entries
                .iter()
                .all(|entry| entry.body["parent_session_id"] == parent)
        );
    }

    #[tokio::test]
    async fn rejects_invalid_creation_without_persisting_a_session_or_consuming_an_alias() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 20).unwrap());
        for body in [
            json!({"name": " "}),
            json!({"workspace_id": ""}),
            json!({"parent_session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAX"}),
        ] {
            let (status, _) = request(&app, "POST", "/v1/sessions", body).await;
            assert!(
                status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
                "{status}"
            );
        }
        let (_, listed) = request(&app, "GET", "/v1/sessions", Value::Null).await;
        assert_eq!(listed, json!([]));
        drop(app);
        let restarted = super::router(Sessions::open(sandbox.path(), 20).unwrap());
        let (status, session) = request(&restarted, "POST", "/v1/sessions", json!({})).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(session["alias"], 20);
    }

    #[tokio::test]
    async fn stop_accepts_an_empty_body_and_rejects_an_invalid_caller_context() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, created) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let id = created["id"].as_str().unwrap();
        let path = format!("/v1/sessions/{id}/stop");
        let (status, _) =
            request(&app, "POST", &path, json!({"parent_session_id": "invalid"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, stopped) = request(&app, "POST", &path, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stopped["state"], "finished");
    }

    #[tokio::test]
    async fn projects_all_lifecycle_events_and_rejects_an_illegal_transition_without_a_log_append()
    {
        use crate::sessions::{CreateSession, Error};
        use ws_core::{
            ids::Ulid,
            log::SessionLog,
            state::{SessionState::*, TransitionError},
        };

        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let parent = sessions.create(CreateSession::default()).unwrap().id;
        let child = sessions
            .create(CreateSession {
                parent_session_id: Some(parent),
                ..Default::default()
            })
            .unwrap()
            .id;
        for to in [
            Running, Blocked, Running, Finished, Reopened, Running, Failed, Reopened, Running,
            Blocked, Finished,
        ] {
            sessions
                .transition(&child.to_string(), to, Some(parent))
                .unwrap();
        }
        let log = SessionLog::new(sandbox.path(), child, Ulid::new());
        let before = log.read().unwrap().entries;
        assert_eq!(
            before
                .iter()
                .map(|entry| entry.event_type.as_str())
                .collect::<Vec<_>>(),
            [
                "session.created",
                "session.running",
                "session.blocked",
                "session.running",
                "session.finished",
                "session.reopened",
                "session.running",
                "session.failed",
                "session.reopened",
                "session.running",
                "session.blocked",
                "session.finished"
            ]
        );
        assert!(
            before
                .iter()
                .all(|entry| entry.body["parent_session_id"] == parent.to_string())
        );
        assert!(matches!(
            sessions.transition(&child.to_string(), Running, Some(parent)),
            Err(Error::Transition(TransitionError {
                from: Finished,
                to: Running
            }))
        ));
        assert_eq!(log.read().unwrap().entries, before);
        let app = super::router(sessions);
        let (status, fetched) =
            request(&app, "GET", &format!("/v1/sessions/{child}"), Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched["state"], "finished");
        assert_eq!(fetched["parent_session_id"], parent.to_string());
    }

    #[tokio::test]
    async fn concurrent_creations_allocate_distinct_consecutive_aliases() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = super::router(Sessions::open(sandbox.path(), 100).unwrap());
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let app = app.clone();
            tasks.push(tokio::spawn(async move {
                request(&app, "POST", "/v1/sessions", json!({})).await
            }));
        }
        let mut aliases = Vec::new();
        for task in tasks {
            let (status, session) = task.await.unwrap();
            assert_eq!(status, StatusCode::CREATED);
            aliases.push(session["alias"].as_i64().unwrap());
        }
        aliases.sort_unstable();
        assert_eq!(
            aliases,
            [100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111]
        );
    }

    async fn request(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(if body.is_null() {
                        Body::empty()
                    } else {
                        Body::from(body.to_string())
                    })
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}
