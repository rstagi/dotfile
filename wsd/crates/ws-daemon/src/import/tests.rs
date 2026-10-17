use std::{fs, path::Path, process::Command};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::from_zsh_with_root;
use crate::{api, sessions::Sessions};

#[tokio::test]
async fn imported_exit_trigger_ignores_exits_before_registration() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    from_zsh_with_root(&source, &state, &source.join("worktrees"), false).unwrap();
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
async fn imported_exit_after_registration_satisfies_the_pending_trigger() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    let mut events = fs::read_to_string(source.join("sessions/1/events")).unwrap();
    events.push_str("{\"event\":\"exit\",\"code\":7}\n");
    fs::write(source.join("sessions/1/events"), events).unwrap();
    from_zsh_with_root(&source, &state, &source.join("worktrees"), false).unwrap();
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let target = sessions.get("#2", None).unwrap().id;
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
        &format!("/v1/inbox?session_id={target}"),
        Value::Null,
    )
    .await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(
        request(&app, "GET", "/v1/triggers", Value::Null).await,
        json!([])
    );
}

#[tokio::test]
async fn imported_done_trigger_reads_output_and_keeps_its_sender() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture_mode(&source, root.path(), "done");
    fs::create_dir(source.join("triggers/archive")).unwrap();
    fs::copy(
        source.join("triggers/2.json"),
        source.join("triggers/archive/2.json"),
    )
    .unwrap();
    fs::write(source.join("triggers/99.json"), "{\"status\":\"fired\"}").unwrap();
    let plan = from_zsh_with_root(&source, &state, &source.join("worktrees"), false).unwrap();
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
    from_zsh_with_root(&source, &state, &source.join("worktrees"), false).unwrap();
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

#[test]
fn imports_only_listed_branch_worktrees_and_their_pins() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let source = sandbox.path().join("legacy");
    let state = sandbox.path().join("state");
    let root = source.join("worktrees");
    let repo = sandbox.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(source.join("sessions")).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "initial",
            "--allow-empty",
        ],
    );
    let branch = root.join("project/feature");
    let detached = root.join("project/detached");
    let outside = sandbox.path().join("outside");
    for (path, args) in [
        (&branch, vec!["-b", "feature"]),
        (&detached, vec!["--detach"]),
        (&outside, vec!["-b", "outside"]),
    ] {
        let mut command = vec!["worktree", "add", "-q"];
        command.extend(args);
        command.push(path.to_str().unwrap());
        git(&repo, &command);
    }
    fs::create_dir_all(root.join("project/unlisted")).unwrap();
    fs::copy(branch.join(".git"), root.join("project/unlisted/.git")).unwrap();
    fs::write(
        source.join("pins"),
        "project/feature\nproject/detached\nproject/unlisted\n",
    )
    .unwrap();
    let preview = from_zsh_with_root(&source, &state, &root, true).unwrap();
    assert_eq!(preview.workspaces.len(), 1);
    assert!(!state.exists());
    assert!(!source.join("sessions/counter.lock").exists());
    assert_eq!(
        fs::read_to_string(source.join("pins")).unwrap(),
        "project/feature\nproject/detached\nproject/unlisted\n"
    );
    from_zsh_with_root(&source, &state, &root, false).unwrap();
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let workspaces = sessions.registry.workspaces().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, "project/feature");
    assert_eq!(
        workspaces[0].body["path"],
        branch.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(workspaces[0].body["pinned"], true);
    let owner = sessions.get("human", None).unwrap().id;
    assert_eq!(workspaces[0].owner_session_id, owner);
    drop(sessions);
    let logs = session_logs(&state);
    let journal = fs::read(state.join("import-zsh.json")).unwrap();
    from_zsh_with_root(&source, &state, &root, false).unwrap();
    assert_eq!(session_logs(&state), logs);
    assert_eq!(fs::read(state.join("import-zsh.json")).unwrap(), journal);
}

#[tokio::test]
async fn imports_spawn_actions_and_resumes_from_the_frozen_workspace_snapshot() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let source = sandbox.path().join("legacy");
    let state = sandbox.path().join("state");
    let root = source.join("worktrees");
    let repo = sandbox.path().join("repo");
    let branch = root.join("project/feature");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "initial",
            "--allow-empty",
        ],
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            branch.to_str().unwrap(),
        ],
    );
    let output = Command::new("zsh")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/import/fixtures/triggers.zsh"
        ))
        .args([source.to_str().unwrap(), "exit", branch.to_str().unwrap()])
        .env("HOME", sandbox.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    from_zsh_with_root(&source, &state, &root, false).unwrap();
    let app = api::router(Sessions::open(&state, 1).unwrap());
    let triggers = request(&app, "GET", "/v1/triggers", Value::Null).await;
    let spawn = triggers
        .as_array()
        .unwrap()
        .iter()
        .find(|trigger| trigger["action"] == "spawn")
        .unwrap();
    assert_eq!(spawn["agent"], "codex");
    assert_eq!(spawn["workspace"], "project/feature");
    assert_eq!(spawn["target"], Value::Null);
    let child = spawn["spawned_session"].as_str().unwrap();
    assert!(child.parse::<ws_core::ids::Ulid>().is_ok());
    assert_eq!(spawn["edges"][0]["source"], child);
    drop(app);
    let logs = session_logs(&state);
    let journal = fs::read(state.join("import-zsh.json")).unwrap();
    let mut incomplete: Value = serde_json::from_slice(&journal).unwrap();
    incomplete["complete"] = json!(false);
    fs::write(state.join("import-zsh.json"), incomplete.to_string()).unwrap();
    fs::rename(&branch, sandbox.path().join("moved")).unwrap();
    fs::write(source.join("triggers/2.json"), "changed after snapshot").unwrap();
    from_zsh_with_root(&source, &state, &root, false).unwrap();
    assert_eq!(session_logs(&state), logs);
    assert_eq!(fs::read(state.join("import-zsh.json")).unwrap(), journal);
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

fn git(repo: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "{arguments:?}: {output:?}");
}
