use std::{fs, path::Path, process::Command};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::from_zsh;
use crate::{api, sessions::Sessions};

#[tokio::test]
async fn imported_exit_trigger_ignores_exits_before_registration() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    from_zsh(&source, &state, false).unwrap();
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let source_id = sessions.get("#1", None).unwrap().id;
    let target_id = sessions.get("#2", None).unwrap().id;
    let app = api::router(sessions);
    let triggers = request(&app, "GET", "/v1/triggers", Value::Null).await;
    assert_eq!(triggers.as_array().unwrap().len(), 1);
    // A different source causes an evaluation, but the old exit must not satisfy it.
    let other = request(&app, "POST", "/v1/sessions", json!({})).await;
    request(
        &app,
        "POST",
        &format!("/v1/sessions/{}/done", other["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/v1/inbox?session_id={target_id}"),
            Value::Null
        )
        .await,
        json!([])
    );
    assert_eq!(
        request(&app, "GET", "/v1/edges", Value::Null)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(app);
    // Pending actions survive a restart and fire exactly once for a new exit.
    let app = api::router(Sessions::open(&state, 1).unwrap());
    request(&app, "POST", "/v1/hooks/ingest", json!({"callback_id":ws_core::ids::Ulid::new(), "session_id":source_id, "observed_at_us":1, "state":"exit", "pane":null, "tmux_socket":null, "tmux_path":null, "payload":{"hook_event_name":"SessionEnd", "exit_code":0}})).await;
    let inbox = request(
        &app,
        "GET",
        &format!("/v1/inbox?session_id={target_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["body"], format!("exited {source_id}"));
    assert_eq!(
        request(&app, "GET", "/v1/triggers", Value::Null).await,
        json!([])
    );
    assert_eq!(
        request(&app, "GET", "/v1/edges", Value::Null).await,
        json!([])
    );
}

#[tokio::test]
async fn imported_done_trigger_reads_output_and_keeps_its_sender() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture_mode(&source, root.path(), "done");
    let plan = from_zsh(&source, &state, false).unwrap();
    assert_eq!(plan.triggers.len(), 2);
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let source_id = sessions.get("#1", None).unwrap().id;
    let target_id = sessions.get("#2", None).unwrap().id;
    let app = api::router(sessions);
    let other = request(&app, "POST", "/v1/sessions", json!({})).await;
    request(
        &app,
        "POST",
        &format!("/v1/sessions/{}/done", other["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    let inbox = request(
        &app,
        "GET",
        &format!("/v1/inbox?session_id={target_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["from_session"], source_id.to_string());
    assert_eq!(
        inbox[0]["body"],
        format!("result {source_id}: completed {{id}} and {{output}}\n")
    );
    drop(app);
    let journal = fs::read(state.join("import-zsh.json")).unwrap();
    let logs = session_logs(&state);
    from_zsh(&source, &state, false).unwrap();
    assert_eq!(fs::read(state.join("import-zsh.json")).unwrap(), journal);
    assert_eq!(session_logs(&state), logs);
    let app = api::router(Sessions::open(&state, 1).unwrap());
    assert_eq!(
        request(&app, "GET", "/v1/triggers", Value::Null)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn session_logs(state: &Path) -> Vec<Vec<u8>> {
    let mut paths = fs::read_dir(state.join("sessions"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("log.jsonl"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| fs::read(path).unwrap())
        .collect()
}

fn capture(source: &Path, home: &Path) {
    capture_mode(source, home, "exit");
}

fn capture_mode(source: &Path, home: &Path, mode: &str) {
    let output = Command::new("zsh")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/import/fixtures/triggers.zsh"
        ))
        .arg(source)
        .arg(mode)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        fs::read_to_string(source.join("triggers/1.json"))
            .unwrap()
            .contains("event_offsets")
    );
}

async fn request(app: &Router, method: &str, path: &str, body: Value) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "{status}: {value}"
    );
    value
}
