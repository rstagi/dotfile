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

#[test]
fn delivery_defaults_to_rust_without_a_selector() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    assert!(
        super::backend::delivery_guard(home.path())
            .unwrap()
            .is_some()
    );
    fs::write(home.path().join("backend"), "zsh\n").unwrap();
    assert!(
        super::backend::delivery_guard(home.path())
            .unwrap()
            .is_none()
    );
}

#[test]
fn incremental_resume_imports_changes_after_the_interrupted_snapshot() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    fs::write(
        source.join("sessions/2/inbox"),
        "{\"message\":\"before crash\"}\n",
    )
    .unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    let target = sessions.get("#2", None).unwrap().id;
    let mut journal: Value =
        serde_json::from_slice(&fs::read(state.join(super::JOURNAL)).unwrap()).unwrap();
    journal["complete"] = json!(false);
    fs::write(state.join(super::JOURNAL), journal.to_string()).unwrap();
    fs::write(
        source.join("sessions/2/inbox"),
        "{\"message\":\"after crash\"}\n",
    )
    .unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    let inbox = sessions.inbox(target).unwrap();
    assert_eq!(inbox.len(), 2);
    assert_eq!(inbox[1].content.body, "after crash");
}

#[test]
fn incremental_import_distinguishes_identical_messages_after_zsh_consumes_an_inbox() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    for expected in 1..=2 {
        let output = Command::new("zsh")
            .args([
                "-fc",
                r#"
            source "$1/ws/ws.sh"
            : > "$WS_SESSIONS/2/inbox"
            with_lock "$WS_SESSIONS/2/inbox" ws_bus_enqueue 2 'same message'
        "#,
                "enqueue",
            ])
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
            .env("HOME", root.path())
            .env("WS_HOME", &source)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
        let target = sessions.get("#2", None).unwrap().id;
        assert_eq!(sessions.inbox(target).unwrap().len(), expected);
    }
}

#[test]
fn incremental_import_keeps_ids_and_imports_replaced_inboxes_and_new_triggers() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    from_zsh_with_root(&source, &state, &source.join("worktrees"), false).unwrap();
    let mut sessions = Sessions::open_with_home(&state, 1, &source).unwrap();
    let target = sessions.get("#2", None).unwrap().id;
    fs::write(
        source.join("sessions/2/inbox"),
        "{\"message\":\"first import\"}\n",
    )
    .unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(sessions.inbox(target).unwrap().len(), 1);
    fs::write(
        source.join("sessions/2/inbox"),
        "{\"message\":\"second import\"}\n",
    )
    .unwrap();
    let mut trigger: Value =
        serde_json::from_slice(&fs::read(source.join("triggers/1.json")).unwrap()).unwrap();
    trigger["template"] = json!("new action {id}");
    fs::write(source.join("triggers/3.json"), trigger.to_string()).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(sessions.get("#2", None).unwrap().id, target);
    let inbox = sessions.inbox(target).unwrap();
    assert_eq!(inbox.len(), 2);
    assert_eq!(inbox[1].content.body, "second import");
    assert_eq!(sessions.triggers.len(), 2);
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(sessions.inbox(target).unwrap().len(), 2);
    assert_eq!(sessions.triggers.len(), 2);
}

#[test]
fn incremental_import_keeps_output_published_by_rust() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture_mode(&source, root.path(), "done");
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    let id = sessions.get("#1", None).unwrap().id;
    let output = state
        .join("sessions")
        .join(id.to_string())
        .join("output.md");
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "completed {id} and {output}\n"
    );
    fs::write(&output, "rust output\n").unwrap();
    sessions
        .append_entry(id, "done", json!({"output":output}))
        .unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "rust output\n");
    // New zsh output after a backend change is imported once.
    fs::write(source.join("sessions/1/output.md"), "zsh again\n").unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "zsh again\n");
    fs::write(&output, "rust again\n").unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), "rust again\n");
}

#[test]
fn incremental_import_cancels_triggers_removed_in_zsh() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(sessions.triggers.len(), 1);
    fs::remove_file(source.join("triggers/1.json")).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert!(sessions.triggers.is_empty());
    assert_eq!(trigger_events(&state, "edge.action_removed"), 1);
    drop(sessions);
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    sessions.restore_triggers().unwrap();
    assert!(sessions.triggers.is_empty());
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(trigger_events(&state, "edge.action_removed"), 1);
}

#[test]
fn incremental_import_completes_triggers_fired_in_zsh() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    capture(&source, root.path());
    let mut sessions = Sessions::open_with_home(&state, 3, &source).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert_eq!(sessions.triggers.len(), 1);
    let mut record: Value =
        serde_json::from_slice(&fs::read(source.join("triggers/1.json")).unwrap()).unwrap();
    record["status"] = json!("fired");
    fs::create_dir(source.join("triggers/archive")).unwrap();
    fs::write(source.join("triggers/archive/1.json"), record.to_string()).unwrap();
    fs::remove_file(source.join("triggers/1.json")).unwrap();
    super::incremental(&mut sessions, &source, &source.join("worktrees")).unwrap();
    assert!(sessions.triggers.is_empty());
    assert_eq!(trigger_events(&state, "edge.action_fired"), 1);
    assert_eq!(trigger_events(&state, "edge.action_removed"), 0);
}

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

fn trigger_events(state: &Path, kind: &str) -> usize {
    session_logs(state)
        .iter()
        .flat_map(|log| log.split(|byte| *byte == b'\n'))
        .filter(|line| !line.is_empty())
        .filter(|line| serde_json::from_slice::<Value>(line).unwrap()["type"] == kind)
        .count()
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
