use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use ws_daemon::{api, sessions::Sessions};

#[tokio::test]
async fn default_child_limit_rejects_the_ninth_child_and_survives_restart() {
    let sandbox = tempfile::tempdir().unwrap();
    let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
    let (_, root) = post(&app, "/v1/sessions", json!({})).await;
    for _ in 0..8 {
        let (status, _) = post(
            &app,
            "/v1/sessions",
            json!({"parent_session_id":root["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, error) = post(
        &app,
        "/v1/sessions",
        json!({"parent_session_id":root["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error["error"]["code"], "max_children_exceeded");
    assert_eq!(error["error"]["limit"], 8);
    assert_eq!(error["error"]["parent_session_id"], root["id"]);
    drop(app);
    let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
    let (status, error) = post(
        &app,
        "/v1/sessions",
        json!({"parent_session_id":root["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error["error"]["code"], "max_children_exceeded");
    let (status, _) = post(&app, "/v1/sessions", json!({})).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn configured_child_limit_counts_live_children_and_keeps_roots_independent() {
    let sandbox = tempfile::tempdir().unwrap();
    std::fs::write(
        sandbox.path().join("config.toml"),
        "[limits]\nmax_children = 1\n",
    )
    .unwrap();
    let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
    let root = sessions.create(Default::default()).unwrap();
    let child = sessions
        .create(ws_daemon::sessions::CreateSession {
            parent_session_id: Some(root.id),
            ..Default::default()
        })
        .unwrap();
    sessions.stop(&child.id.to_string(), None).unwrap();
    let app = api::router(sessions);
    let (status, live) = post(&app, "/v1/sessions", json!({"parent_session_id":root.id})).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, error) = post(&app, "/v1/sessions", json!({"parent_session_id":root.id})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error["error"]["code"], "max_children_exceeded");
    assert_eq!(error["error"]["limit"], 1);
    assert!(live["id"].is_string());
    let (status, other) = post(&app, "/v1/sessions", json!({})).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = post(
        &app,
        "/v1/sessions",
        json!({"parent_session_id":other["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn depth_limit_counts_edges_from_each_root_and_accepts_configuration() {
    for (config, limit) in [
        (None, 3),
        (Some("[limits]\nmax_depth = 1\n"), 1),
        (Some("[limits]\nmax_depth = 0\n"), 0),
    ] {
        let sandbox = tempfile::tempdir().unwrap();
        if let Some(config) = config {
            std::fs::write(sandbox.path().join("config.toml"), config).unwrap();
        }
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, root) = post(&app, "/v1/sessions", json!({})).await;
        let mut parent = root;
        for _ in 0..limit {
            let (status, child) = post(
                &app,
                "/v1/sessions",
                json!({"parent_session_id":parent["id"]}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            parent = child;
        }
        let (status, error) = post(
            &app,
            "/v1/sessions",
            json!({"parent_session_id":parent["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error["error"]["code"], "max_depth_exceeded");
        assert_eq!(error["error"]["limit"], limit);
        assert_eq!(error["error"]["parent_session_id"], parent["id"]);
        let (status, _) = post(&app, "/v1/sessions", json!({})).await;
        assert_eq!(status, StatusCode::CREATED);
    }
}

#[tokio::test]
async fn headless_sessions_and_phases_check_limits_before_starting_a_process() {
    for (setting, code) in [
        ("max_children", "max_children_exceeded"),
        ("max_depth", "max_depth_exceeded"),
    ] {
        let sandbox = tempfile::tempdir().unwrap();
        std::fs::write(
            sandbox.path().join("config.toml"),
            format!("[limits]\n{setting} = 0\n"),
        )
        .unwrap();
        let app = api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, root) = post(&app, "/v1/sessions", json!({})).await;
        let marker = sandbox.path().join("child-started");
        for (path, body) in [
            (
                "/v1/sessions",
                json!({"headless":true,"command":["/usr/bin/touch",marker],"cwd":sandbox.path(),"parent_session_id":root["id"]}),
            ),
            (
                "/v1/phases/spawn",
                json!({"parent_session_id":root["id"],"phase":{"command":["/usr/bin/touch",marker]},"repo":sandbox.path(),"base":"main","caps_required":[],"max_runtime":30}),
            ),
        ] {
            let (status, error) = post(&app, path, body).await;
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{path}: {error}");
            assert_eq!(error["error"]["code"], code);
            assert!(!marker.exists());
        }
    }
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
