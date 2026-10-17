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
struct SessionResponse {
    #[serde(flatten)]
    session: Session,
    parent_session_id: Option<SessionId>,
}

pub(crate) struct ApiError(pub(crate) Error);

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallContext {
    pub(crate) parent_session_id: Option<SessionId>,
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
        .merge(crate::hooks::router())
        .merge(crate::workspaces::router())
        .merge(crate::agents::router())
        .route("/v1/sessions", get(list).post(create))
        .route("/v1/sessions/{id}", get(fetch))
        .route("/v1/sessions/{id}/stop", axum::routing::post(stop))
        .route("/v1/sessions/{id}/name", axum::routing::post(rename))
        .with_state(store)
}

async fn create(
    State(store): State<Store>,
    Json(request): Json<CreateSession>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    let session = call(store, move |sessions| sessions.create(request)).await?;
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
        call(store, move |sessions| {
            sessions.stop(&id, context.parent_session_id)
        })
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
        operation(&mut sessions)
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
            Error::NameTaken(_) => (StatusCode::CONFLICT, "name_taken"),
            Error::Transition(_) => (StatusCode::CONFLICT, "invalid_transition"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
        };
        let mut error = json!({"code": code, "message": self.0.to_string()});
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
                "session.get"
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
