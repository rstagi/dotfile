mod common;

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Sandbox {
    home: common::SandboxRoot,
    socket: PathBuf,
    server: String,
    listener: UnixListener,
    _daemon: common::Daemon,
}

#[test]
fn start_api_rejects_invalid_places_and_names_before_creating_resources() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path();
    let file = directory.join("document");
    fs::write(&file, "text").unwrap();
    for body in [
        json!({"agent":"claude"}),
        json!({"cwd":directory, "scratch":true, "agent":"claude"}),
        json!({"scratch":false, "agent":"claude"}),
        json!({"scratch":true, "agent":"unknown"}),
        json!({"cwd":file, "agent":"claude"}),
        json!({"cwd":directory.join("missing"), "agent":"claude"}),
        json!({"scratch":true, "agent":"claude", "name":"../escape"}),
        json!({"scratch":true, "agent":"claude", "name":"me"}),
        json!({"scratch":true, "agent":"claude", "name":""}),
    ] {
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/sessions/start",
            &body.to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 400"), "{body}: {response}");
    }
    assert!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!directory.join(".ws/scratch").exists());
    assert_eq!(
        sandbox.tmux(&["list-sessions", "-F", "#{session_name}"]),
        "keepalive"
    );
}

#[test]
fn directory_agents_share_a_tmux_session_and_restart_in_the_same_place() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("plain-dir");
    fs::create_dir(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let alias = sandbox.home.path().join("directory-alias");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    let mut agents = Vec::new();
    let mut tmux_session = String::new();
    for cwd in [&directory, &alias] {
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/sessions/start",
            &json!({"cwd":cwd, "agent":"grok", "prompt":"First turn only."}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let (agent, stream) = sandbox.capture();
        let id = agent["id"].as_str().unwrap();
        let window = sandbox.window(id);
        let grouped = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{session_name}"]);
        if tmux_session.is_empty() {
            tmux_session = grouped.clone();
        }
        assert_eq!(grouped, tmux_session);
        agents.push((agent, stream));
    }
    let identity = agents[0].0["id"].as_str().unwrap().to_owned();
    let id = identity.as_str();
    let old_window = sandbox.window(id);
    // Completion preserves the window. A later clean process exit requires a new launch.
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", id)
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    assert_eq!(sandbox.window(id), old_window);
    agents[0].1.write_all(b"0").unwrap();
    sandbox.wait_for(|| !sandbox.events(id, "exit").is_empty());
    let sent = sandbox.output(&["send", id, "Resume this directory."]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    let (restarted, _running) = sandbox.capture();
    assert_eq!(restarted["id"], id);
    assert_eq!(restarted["cwd"], directory.to_str().unwrap());
    assert!(
        !restarted["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("First turn only."))
    );
    let window = sandbox.window(id);
    assert_ne!(window, old_window);
    assert_eq!(
        sandbox.tmux(&["display-message", "-p", "-t", &window, "#{session_name}"]),
        tmux_session
    );
    assert!(sandbox.json(&["show", id, "--json"])["workspace_id"].is_null());
    assert!(
        sandbox
            .json(&["list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn start_api_creates_named_and_unnamed_scratch_directories() {
    let sandbox = Sandbox::new();
    for name in [Some("draft"), None] {
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/sessions/start",
            &json!({"scratch":true, "agent":"claude", "name":name}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let started: Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let id = started["session_id"].as_str().unwrap();
        let suffix = &id[id.len() - 8..];
        let leaf = name.map_or_else(|| suffix.to_owned(), |name| format!("{name}-{suffix}"));
        let expected = sandbox
            .home
            .path()
            .join(".ws/scratch")
            .join(leaf)
            .canonicalize()
            .unwrap();
        let (agent, _running) = sandbox.capture();
        assert_eq!(agent["cwd"], expected.to_str().unwrap());
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(session["cwd"], expected.to_str().unwrap());
        assert!(session["workspace_id"].is_null());
        assert!(
            sandbox
                .json(&["list", "--json"])
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn mcp_session_create_starts_an_interactive_agent_in_a_directory() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};

    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("mcp notes");
    fs::create_dir(&directory).unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let parent: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let mut command = tokio::process::Command::from(sandbox.command());
    command
        .arg("mcp")
        .env("WS_SESSION_ID", parent["id"].as_str().unwrap())
        .current_dir(sandbox.home.path());
    let client = tokio::time::timeout(
        Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("session_create").with_arguments(
                json!({"cwd":"mcp notes", "name":"mcp-notes"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let session: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(result.structured_content.as_ref(), Some(&session));
    let (agent, _running) = sandbox.capture();
    assert_eq!(session["id"], agent["id"]);
    assert_eq!(
        session["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(agent["cwd"], session["cwd"]);
    assert_eq!(session["name"], "mcp-notes");
    assert_eq!(session["parent_session_id"], parent["id"]);
    assert!(session["workspace_id"].is_null());
    assert!(session["body"]["spawned"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), json!([]));
    let rejected = client
        .call_tool(
            CallToolRequestParams::new("session_create").with_arguments(
                json!({"cwd":"mcp notes", "workspace_id":"unused"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(rejected.is_error, Some(true), "{rejected:?}");
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    client.cancel().await.unwrap();
}

#[test]
fn new_passes_name_and_prompt_to_workspace_and_directory_starts() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let parent: Value = serde_json::from_str(
        common::request(&sandbox.socket, "POST", "/v1/sessions", "{}")
            .split_once("\r\n\r\n")
            .unwrap()
            .1,
    )
    .unwrap();
    let parent = parent["id"].as_str().unwrap();
    let starts: [(&[&str], &str, &str); 3] = [
        (
            &["--repo", repo.to_str().unwrap(), "--branch", "feature"],
            "claude",
            "api-feature",
        ),
        (&["--scratch", "--agent", "codex"], "codex", "scratch-notes"),
        (
            &["--cwd", repo.to_str().unwrap(), "--agent", "grok"],
            "grok",
            "repo-notes",
        ),
    ];
    for (place, agent, name) in starts {
        let prompt = format!("Start {name} now.");
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", parent)
            .arg("new")
            .args(place)
            .args(["--name", name, "--prompt", &prompt, "--detach"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let (capture, _running) = sandbox.capture();
        assert_eq!(capture["agent"], agent);
        let argv: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        assert!(argv.contains(&prompt), "{argv:?}");
        let session = sandbox.json(&["show", name, "--json"]);
        assert_eq!(session["id"], capture["id"]);
        assert_eq!(session["body"]["parent_session_id"], parent);
        assert_eq!(session["workspace_id"].is_null(), agent != "claude");
    }
    let output = sandbox
        .command()
        .env("WS_SESSION_ID", parent)
        .args([
            "new",
            "--scratch",
            "--no-parent",
            "--name",
            "top-level",
            "--detach",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let _running = sandbox.capture();
    let session = sandbox.json(&["show", "top-level", "--json"]);
    assert!(session["body"]["parent_session_id"].is_null(), "{session}");
    let rejected = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "other",
        "--name",
        "api-feature",
        "--detach",
    ]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("name already in use"),
        "{rejected:?}"
    );
    assert!(!sandbox.home.path().join("worktrees/api/other").exists());
}

#[test]
fn a_new_tmux_session_keeps_the_agent_identity_out_of_its_environment() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feature",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let (agent, _running) = sandbox.capture();
    let window = sandbox.window(agent["id"].as_str().unwrap());
    let session = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{session_name}"]);
    let environment = sandbox.tmux(&["show-environment", "-t", session.trim()]);
    assert!(
        !environment
            .lines()
            .any(|line| line.starts_with("WS_SESSION_ID=")),
        "{environment}"
    );
}

#[tokio::test]
async fn mcp_session_create_starts_workspace_and_scratch_agents_with_prompts() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};

    let sandbox = Sandbox::new();
    sandbox.repo();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let parent: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let mut command = tokio::process::Command::from(sandbox.command());
    command
        .arg("mcp")
        .env("WS_SESSION_ID", parent["id"].as_str().unwrap())
        .current_dir(sandbox.home.path());
    let client = tokio::time::timeout(
        Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let call = async |arguments: Value| {
        client
            .call_tool(
                CallToolRequestParams::new("session_create")
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap()
    };
    let created = call(json!({"repo":"api", "branch":"feature", "agent":"codex", "name":"api-work", "prompt":"Fix the login bug."})).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    let session = created.structured_content.unwrap();
    let (capture, _workspace) = sandbox.capture();
    assert_eq!(capture["id"], session["id"]);
    assert_eq!(capture["agent"], "codex");
    assert!(
        capture["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("Fix the login bug."))
    );
    assert_eq!(session["name"], "api-work");
    assert_eq!(session["workspace_id"], "api/feature");
    assert_eq!(session["cwd"], sandbox.worktree().to_str().unwrap());
    assert_eq!(session["parent_session_id"], parent["id"]);
    assert!(session["alias"].is_i64());
    let workspaces = sandbox.json(&["list", "--json"]);
    assert_eq!(workspaces[0]["id"], "api/feature");
    let scratch = call(json!({"scratch":true, "prompt":"Take notes."})).await;
    assert_ne!(scratch.is_error, Some(true), "{scratch:?}");
    let scratch = scratch.structured_content.unwrap();
    let (capture, _scratch) = sandbox.capture();
    assert_eq!(capture["agent"], "claude");
    assert_eq!(capture["argv"][0], "Take notes.");
    assert!(scratch["workspace_id"].is_null());
    assert!(scratch["cwd"].as_str().unwrap().contains("/.ws/scratch/"));
    assert_eq!(scratch["parent_session_id"], parent["id"]);
    for arguments in [
        json!({"cwd":".", "scratch":true}),
        json!({"repo":"api", "cwd":"."}),
        json!({"repo":"api", "workspace_id":"api/feature"}),
        json!({"branch":"feature"}),
        json!({"repo":"api"}),
        json!({"prompt":"No place."}),
        json!({"agent":"codex"}),
        json!({"agent":"unknown", "scratch":true}),
        json!({"headless":true, "command":["/usr/bin/true"], "prompt":"x"}),
        json!({"headless":true, "command":["/usr/bin/true"], "scratch":true}),
    ] {
        let rejected = call(arguments.clone()).await;
        assert_eq!(rejected.is_error, Some(true), "{arguments}: {rejected:?}");
    }
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        3
    );
    client.cancel().await.unwrap();
}

#[test]
fn session_create_starts_an_interactive_agent_in_a_directory() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("create notes");
    fs::create_dir(&directory).unwrap();
    let result = sandbox
        .command()
        .current_dir(sandbox.home.path())
        .args([
            "session",
            "create",
            "--cwd",
            "create notes",
            "--name",
            "notes",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let created: Value = serde_json::from_slice(&result.stdout).unwrap();
    let id = created["id"].as_str().unwrap();
    let (agent, _running) = sandbox.capture();
    assert_eq!(agent["id"], id);
    assert_eq!(agent["agent"], "claude");
    assert_eq!(
        agent["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(created["cwd"], agent["cwd"]);
    assert_eq!(created["name"], "notes");
    assert!(created["workspace_id"].is_null());
    assert!(created["body"]["spawned"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), json!([]));
    let window = sandbox.window(id);
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", id)
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    assert_eq!(sandbox.window(id), window);
}

#[test]
#[cfg(target_os = "macos")]
fn agent_priority_clamps_interactive_agents() {
    let inherited_clamp = common::test_process_has_inherited_clamp();
    // macOS reports a background clamp as scheduling priority 4.
    for (config, clamped) in [
        ("[agent]\npriority = \"background\"\n", true),
        ("[agent]\npriority = \"normal\"\n", false),
    ] {
        let sandbox = Sandbox::with_config(config);
        let result = sandbox
            .command()
            .current_dir(sandbox.home.path())
            .args(["session", "create", "--cwd", ".", "--json"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        let (agent, _running) = sandbox.capture();
        if !inherited_clamp {
            assert_eq!(agent["pri"] == "4", clamped, "{config}: {agent}");
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
fn session_priority_restarts_the_agent_in_its_window_and_resumes_the_conversation() {
    let inherited_clamp = common::test_process_has_inherited_clamp();
    let sandbox = Sandbox::with_config("[agent]\npriority = \"normal\"\n");
    let result = sandbox
        .command()
        .current_dir(sandbox.home.path())
        .args(["session", "create", "--cwd", ".", "--json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let (first, _first) = sandbox.capture();
    let id = first["id"].as_str().unwrap().to_owned();
    if !inherited_clamp {
        assert_ne!(first["pri"], "4", "{first}");
    }
    let window = sandbox.window(&id);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/hooks/ingest",
        &json!({
            "session_id": id, "callback_id": ws_core::ids::Ulid::new(),
            "observed_at_us": ws_daemon::hooks::receipt_time_us().unwrap(),
            "state": "stop", "pane": pane, "tmux_socket": sandbox.server,
            "payload": {"hook_event_name":"Stop", "session_id":"resume-claude"}
        })
        .to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");

    let changed = sandbox.json(&["session", "priority", &id, "next", "--json"]);
    assert_eq!(changed["body"]["priority"], "background");
    let (second, _second) = sandbox.capture();
    assert_eq!(second["id"], id.as_str());
    if !inherited_clamp {
        assert_eq!(second["pri"], "4", "{second}");
    }
    assert_eq!(second["pane"], pane.as_str());
    assert!(
        second["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|args| args == [json!("--resume"), json!("resume-claude")]),
        "{second}"
    );
    assert_eq!(sandbox.window(&id), window);
    let session = sandbox.json(&["show", &id, "--json"]);
    assert_eq!(session["state"], "running");
    assert_eq!(session["body"]["priority"], "background");
    assert!(sandbox.events(&id, "exit").is_empty());

    let changed = sandbox.json(&["session", "priority", &id, "normal", "--json"]);
    assert_eq!(changed["body"]["priority"], "normal");
    let (third, _third) = sandbox.capture();
    if !inherited_clamp {
        assert_ne!(third["pri"], "4", "{third}");
    }
    assert_eq!(sandbox.events(&id, "session.priority").len(), 2);
}

#[test]
fn start_api_runs_an_agent_in_a_directory_without_registering_a_workspace() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes and plans");
    fs::create_dir(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let before = sandbox.json(&["list", "--json"]);
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/sessions/start",
        &json!({"cwd":directory, "agent":"codex", "name":"notes", "prompt":"Read these notes."})
            .to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let started: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let id = started["session_id"].as_str().unwrap();
    let (agent, _running) = sandbox.capture();
    assert_eq!(agent["id"], id);
    assert_eq!(agent["agent"], "codex");
    assert_eq!(agent["cwd"], directory.to_str().unwrap());
    assert!(
        agent["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("Read these notes."))
    );
    let session = sandbox.json(&["show", id, "--json"]);
    assert_eq!(session["name"], "notes");
    assert_eq!(session["cwd"], directory.to_str().unwrap());
    assert!(session["workspace_id"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), before);
    let window = sandbox.window(id);
    assert_eq!(started["window_id"], window);
    assert_eq!(
        sandbox.tmux(&["show-option", "-wqv", "-t", &window, "@ws_path"]),
        directory.to_str().unwrap()
    );
    let tmux_session = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{session_name}"]);
    assert!(
        tmux_session.starts_with("notes and plans-"),
        "{tmux_session}"
    );
    assert_eq!(tmux_session.len(), "notes and plans-".len() + 6);
    assert_eq!(
        sandbox.tmux(&["show-option", "-qv", "-t", &tmux_session, "@ws_path"]),
        directory.to_str().unwrap()
    );
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", id)
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    assert_eq!(sandbox.window(id), window);
    assert_eq!(
        sandbox.events(id, "session.created")[0].body["spawned"].as_bool(),
        None
    );
    let sent = sandbox.output(&["send", id, "Continue these notes."]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
    assert_eq!(sandbox.window(id), window);
}

#[test]
fn a_session_window_prepares_an_agent_without_a_workspace() {
    let (sandbox, id, window, pane) = session_window_sandbox();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/launch",
        &json!({"id":id, "pane":pane}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let launch: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(launch["directory"], sandbox.home.path().to_str().unwrap());
    assert_eq!(launch["prepared"]["env"]["WS_SESSION_ID"], id);
    assert_eq!(sandbox.window(&id), window);
}

#[test]
fn a_session_window_owns_hooks_without_a_workspace_or_window_tag() {
    let (sandbox, id, window, pane) = session_window_sandbox();
    sandbox.tmux(&["set-option", "-wu", "-t", &window, "@ws_session"]);
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/hooks/ingest",
        &json!({
            "callback_id":ws_core::ids::Ulid::new(), "session_id":id,
            "observed_at_us":ws_daemon::hooks::receipt_time_us().unwrap(),
            "state":"working", "pane":pane, "tmux_socket":sandbox.server,
            "payload":{"hook_event_name":"UserPromptSubmit"}
        })
        .to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert_eq!(
        sandbox.tmux(&["show-option", "-wqv", "-t", &window, "@ws_state"]),
        "working"
    );
    assert_eq!(sandbox.events(&id, "turn.started").len(), 1);
}

#[test]
fn an_agent_launch_records_its_session_window() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feature",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let (agent, _running) = sandbox.capture();
    let id = agent["id"].as_str().unwrap();
    let events = sandbox.events(id, "session.window");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].body["window_id"], sandbox.window(id));
    assert_eq!(events[0].body["pane"], agent["pane"]);
}

#[test]
fn stop_closes_a_tab_sessions_window_and_agent_process() {
    let (sandbox, id, window, pane) = session_window_sandbox();
    let pid = sandbox.tmux(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
    let output = sandbox.output(&["session", "stop", &id]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "finished");
    assert!(
        !sandbox
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .lines()
            .any(|w| w == window)
    );
    sandbox.wait_for(|| {
        !Command::new("/bin/kill")
            .args(["-0", &pid])
            .output()
            .unwrap()
            .status
            .success()
    });
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
}

#[test]
fn a_completed_session_reopens_its_window_without_a_workspace() {
    let (sandbox, id, window, _) = session_window_sandbox();
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", &id)
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    let sent = sandbox.output(&["send", &id, "Continue this session."]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "running");
    assert_eq!(sandbox.window(&id), window);
    assert!(
        sandbox
            .json(&["list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_stopped_session_restarts_without_creating_a_workspace() {
    let (sandbox, id, old_window, _) = session_window_sandbox();
    assert!(sandbox.output(&["session", "stop", &id]).status.success());
    let sent = sandbox.output(&["send", &id, "Restart this session."]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "running");
    let (agent, _running) = sandbox.capture();
    assert_eq!(agent["id"], id);
    assert_ne!(sandbox.window(&id), old_window);
    assert_eq!(sandbox.events(&id, "session.window").len(), 2);
    assert!(sandbox.json(&["show", &id, "--json"])["workspace_id"].is_null());
    assert!(
        sandbox
            .json(&["list", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stop_preserves_a_window_that_belongs_to_another_session() {
    let (sandbox, id, window, _) = session_window_sandbox();
    sandbox.tmux(&[
        "set-option",
        "-w",
        "-t",
        &window,
        "@ws_session",
        "another-session",
    ]);
    assert!(sandbox.output(&["session", "stop", &id]).status.success());
    assert!(
        sandbox
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .lines()
            .any(|w| w == window)
    );
}

#[test]
fn an_agent_exit_preserves_a_user_tab_without_a_workspace() {
    let (sandbox, id, window, pane) = session_window_sandbox();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({
            "id":id, "pane":pane, "code":0, "user_tab":true
        })
        .to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "finished");
    assert!(
        sandbox
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .lines()
            .any(|w| w == window)
    );
    assert!(
        sandbox
            .tmux(&["show-option", "-wqv", "-t", &window, "@ws_session"])
            .is_empty()
    );
}

#[test]
fn a_failed_agent_exit_fails_the_session_and_a_message_reopens_it() {
    let (sandbox, id, _window, pane) = session_window_sandbox();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({"id":id, "pane":pane, "code":1, "user_tab":false}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let shown = sandbox.json(&["show", &id, "--json"]);
    assert_eq!(shown["state"], "failed", "{shown}");
    assert_eq!(shown["exit_code"], 1, "{shown}");
    let waited = sandbox.output(&["wait", &id, "--for", "state=failed", "--timeout", "5"]);
    assert!(waited.status.success(), "{waited:?}");
    let sent = sandbox.output(&["send", &id, "Try again."]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "running");
    let (agent, _running) = sandbox.capture();
    assert_eq!(agent["id"], id);
}

#[test]
fn a_late_agent_exit_keeps_a_finished_session_finished() {
    let (sandbox, id, _window, pane) = session_window_sandbox();
    assert!(sandbox.output(&["session", "stop", &id]).status.success());
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({"id":id, "pane":pane, "code":1, "user_tab":false}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(sandbox.json(&["show", &id, "--json"])["state"], "finished");
}

#[test]
fn new_from_a_session_pane_without_an_agent_identity_starts_a_top_level_session() {
    let (sandbox, id, _window, pane) = session_window_sandbox();
    let output = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .args(["new", "--scratch", "--name", "from-pane", "--detach"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let _running = sandbox.capture();
    let session = sandbox.json(&["show", "from-pane", "--json"]);
    assert!(
        session["body"]["parent_session_id"].is_null(),
        "The pane session {id} must not become the parent: {session}"
    );
}

fn session_window_sandbox() -> (Sandbox, String, String, String) {
    let id = ws_core::ids::Ulid::new().to_string();
    let mut window = String::new();
    let mut pane = String::new();
    let sandbox = Sandbox::with_setup("", |home, server| {
        let output = Command::new("tmux")
            .args([
                "-S",
                server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "session-window",
                "-P",
                "-F",
                "#{window_id}\t#{pane_id}",
                "/bin/cat",
            ])
            .env("HOME", home)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let ids = String::from_utf8(output.stdout).unwrap();
        let (w, p) = ids.trim_end().split_once('\t').unwrap();
        window = w.into();
        pane = p.into();
        for (key, value) in [
            ("@ws_session", id.as_str()),
            ("@ws_pane", p),
            ("@ws_state", "idle"),
        ] {
            assert!(
                Command::new("tmux")
                    .args(["-S", server, "set-option", "-w", "-t", w, key, value])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let state = home.join("state");
        let device = ws_core::device::Device::init(&state, "test").unwrap();
        let log = ws_core::log::SessionLog::new(&state, id.parse().unwrap(), device.device_id);
        log.append(
            0,
            "session.created",
            json!({"alias":1, "agent":"claude", "cwd":home}),
        )
        .unwrap();
        log.append(0, "session.window", json!({"window_id":w, "pane":p}))
            .unwrap();
        log.append(0, "session.running", json!({})).unwrap();
    });
    (sandbox, id, window, pane)
}

#[test]
fn an_agent_launch_waits_for_its_workspace_commit() {
    let sandbox = Sandbox::with_launch_observation();
    let repo = sandbox.repo();
    let gate = UnixListener::bind(sandbox.home.path().join("commit-gate.sock")).unwrap();
    gate.set_nonblocking(true).unwrap();
    let mut running = Vec::new();
    for arguments in [
        vec![
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "feature",
            "--detach",
        ],
        vec!["add", "api/feature", "--detach"],
    ] {
        let observed = sandbox.home.path().join("launch-requested");
        let _ = fs::remove_file(&observed);
        let child = sandbox
            .command()
            .args(arguments)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut commit = None;
        sandbox.wait_for(|| match gate.accept() {
            Ok((stream, _)) => {
                commit = Some(stream);
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("{error}"),
        });
        sandbox.wait_for(|| observed.exists());
        commit.unwrap().write_all(b"c").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let (agent, connection) = sandbox.capture();
        let id = agent["id"].as_str().unwrap();
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        assert!(!sandbox.window(id).is_empty());
        running.push(connection);
    }
}

#[test]
fn a_failed_workspace_commit_releases_the_session_and_slot() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feature",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let (owner, _running) = sandbox.capture();
    let slots = fs::read_to_string(sandbox.slots()).unwrap();
    let real_tmux = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("tmux"))
        .find(|path| path.is_file())
        .unwrap();
    let wrapper = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&wrapper, format!(
        "#!/bin/sh\ncase \" $* \" in *' rename-window '*) echo 'synthetic commit failure' >&2; exit 1;; esac\nexec {} \"$@\"\n",
        shell_words::quote(&real_tmux.to_string_lossy()),
    )).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let output = sandbox.output(&["add", "api/feature", "--detach"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("synthetic commit failure"));
    let sessions = sandbox.json(&["sessions", "--json"]);
    let failed: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|session| session["body"]["agent"] == "claude" && session["id"] != owner["id"])
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["state"], "failed");
    let preparation = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/launch",
        &json!({"id": failed[0]["id"], "pane": owner["pane"]}).to_string(),
    );
    assert!(
        preparation.contains("This pane does not own the agent session."),
        "{preparation}"
    );
    assert_eq!(fs::read_to_string(sandbox.slots()).unwrap(), slots);
    assert_eq!(
        sandbox
            .tmux(&["list-windows", "-t", "api/feature", "-F", "#{window_id}"])
            .lines()
            .count(),
        1
    );

    // The next launch can reuse the released slot and complete its preparation.
    fs::remove_file(wrapper).unwrap();
    let output = sandbox.output(&["add", "api/feature", "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let (agent, _new_running) = sandbox.capture();
    let id = agent["id"].as_str().unwrap();
    assert_ne!(agent["id"], failed[0]["id"]);
    assert_eq!(sandbox.json(&["show", id, "--json"])["body"]["slot"], 2);
}

#[test]
fn an_exit_self_trigger_reopens_after_the_old_launch_retires() {
    for code in [0, 7] {
        let sandbox = Sandbox::new();
        let repo = sandbox.repo();
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    "feature",
                    "--detach"
                ])
                .status
                .success()
        );
        let (owner, mut old_agent) = sandbox.capture();
        let id = owner["id"].as_str().unwrap();
        let old_window = sandbox.window(id);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["on", id, "exit", "--send-to", "me", "continue after exit"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let end_hook = common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id":id, "callback_id":ws_core::ids::Ulid::new(),
                "observed_at_us":ws_daemon::hooks::receipt_time_us().unwrap(), "state":"exit",
                "pane":owner["pane"], "tmux_socket":sandbox.server,
                "payload":{"hook_event_name":"SessionEnd", "exit_code":0}
            })
            .to_string(),
        );
        assert!(end_hook.starts_with("HTTP/1.1 201"), "{end_hook}");
        assert!(sandbox.events(id, "message.sent").is_empty());
        old_agent
            .write_all(if code == 0 { b"0" } else { b"7" })
            .unwrap();
        let (reopened, _running) = sandbox.capture();
        assert_eq!(reopened["id"], id);
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        assert_ne!(sandbox.window(id), old_window);
        assert_eq!(sandbox.events(id, "message.sent").len(), 1);
        let waited = sandbox.output(&["wait", id, "--for", "event=message.sent", "--timeout", "0"]);
        assert_eq!(waited.status.code(), Some(1), "{waited:?}");
        let waited = sandbox.output(&[
            "wait",
            id,
            "--for",
            "event=session.running",
            "--timeout",
            "0",
        ]);
        assert!(waited.status.success(), "{waited:?}");
    }
}

#[test]
fn failed_trigger_launch_releases_its_slot_and_keeps_diagnostics() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (owner, _running) = sandbox.capture();
    let owner_id = owner["id"].as_str().unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let source_id = source["id"].as_str().unwrap();
    let output = sandbox
        .command()
        .env("WS_SESSION_ID", owner_id)
        .args(["on", source_id, "done", "--spawn", "claude", "continue"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let tid = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let real_tmux = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("tmux"))
        .find(|path| path.is_file())
        .unwrap();
    let wrapper = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&wrapper, format!("#!/bin/sh\ncase \" $* \" in *' new-window '*|*' new-session '*) echo 'synthetic launch failure' >&2; exit 1;; esac\nexec {} \"$@\"\n", shell_words::quote(&real_tmux.to_string_lossy()))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let done = format!("/v1/sessions/{source_id}/done");
    for _ in 0..2 {
        assert!(common::request(&sandbox.socket, "POST", &done, "{}").starts_with("HTTP/1.1 200"));
    }
    assert_eq!(sandbox.json(&["triggers", "--json"]), json!([]));
    let sessions = sandbox.json(&["sessions", "--json"]);
    let children: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|session| session["body"]["trigger_id"] == tid)
        .collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["state"], "failed");
    assert!(
        !fs::read_to_string(sandbox.slots())
            .unwrap()
            .contains(children[0]["id"].as_str().unwrap())
    );
    let failures = sandbox.events(owner_id, "edge.action_failed");
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].body["error"]
            .as_str()
            .unwrap()
            .contains("synthetic launch failure")
    );
}

#[test]
fn terminal_spawn_trigger_uses_the_current_directory() {
    let sandbox = Sandbox::new();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = source["id"].as_str().unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{id}/done"),
        "{}",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let result = sandbox
        .command()
        .current_dir(sandbox.home.path())
        .args(["on", id, "done", "--spawn", "claude", "Continue."])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let (child, _running) = sandbox.capture();
    assert_eq!(
        child["cwd"],
        sandbox
            .home
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(
        sandbox.json(&["show", child["id"].as_str().unwrap(), "--json"])["workspace_id"].is_null()
    );
    assert_eq!(sandbox.json(&["list", "--json"]), json!([]));
}

#[test]
fn four_orchestrator_continuations_keep_the_loop_parent() {
    let loop_id = ws_core::ids::SessionId::new();
    let sandbox = Sandbox::with_setup("", |home, _| {
        let state = home.join("state");
        let device = ws_core::device::Device::init(&state, "test").unwrap();
        ws_core::log::SessionLog::new(&state, loop_id, device.device_id)
            .append(0, "session.created", json!({"alias":1,"kind":"loop"}))
            .unwrap();
    });
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/sessions/start",
        &json!({"cwd":sandbox.home.path(),"agent":"codex","parent_session_id":loop_id}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let (mut owner, mut running) = sandbox.capture();
    for _ in 0..4 {
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", owner["id"].as_str().unwrap())
            .args(["on", "me", "exit", "--spawn", "codex", "Continue the loop."])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        running.write_all(b"0").unwrap();
        let (child, child_running) = sandbox.capture();
        let session = sandbox.json(&["show", child["id"].as_str().unwrap(), "--json"]);
        assert_eq!(session["parent_session_id"], loop_id.to_string());
        owner = child;
        running = child_running;
    }
    running.write_all(b"0").unwrap();
    let output = sandbox.output(&[
        "wait",
        owner["id"].as_str().unwrap(),
        "--for",
        "exit",
        "--timeout",
        "30",
    ]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn trigger_spawn_uses_the_owners_directory_without_a_workspace() {
    let mut sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("owner notes");
    fs::create_dir(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/sessions/start",
        &json!({"cwd":directory, "agent":"claude"}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let (owner, _running) = sandbox.capture();
    let owner_id = owner["id"].as_str().unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let source_id = source["id"].as_str().unwrap();
    let output = sandbox
        .command()
        .env("WS_SESSION_ID", owner_id)
        .args([
            "on",
            source_id,
            "done",
            "--spawn",
            "codex",
            "Continue {id}: {output}",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let trigger = sandbox.json(&["triggers", "--json"]);
    assert_eq!(trigger[0]["cwd"], directory.to_str().unwrap());
    assert!(trigger[0]["workspace"].is_null());
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.json(&["triggers", "--json"]), trigger);
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{source_id}/done"),
        r#"{"output":"the result"}"#,
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let (child, _child_running) = sandbox.capture();
    let child_id = child["id"].as_str().unwrap();
    assert_eq!(child["cwd"], directory.to_str().unwrap());
    assert!(
        child["argv"]
            .as_array()
            .unwrap()
            .contains(&json!(format!("Continue {source_id}: the result")))
    );
    let session = sandbox.json(&["show", child_id, "--json"]);
    assert_eq!(session["body"]["parent_session_id"], owner_id);
    assert!(session["workspace_id"].is_null());
    assert_eq!(session["body"]["spawned"], true);
    assert_eq!(session["cwd"], directory.to_str().unwrap());
    assert_eq!(sandbox.json(&["list", "--json"]), json!([]));
    let window = sandbox.window(child_id);
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", child_id)
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    assert!(
        !sandbox
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .lines()
            .any(|id| id == window)
    );
}

#[test]
fn trigger_spawn_preserves_a_literal_prompt_and_closes_the_child_on_done() {
    // Golden scenarios: ws/tests/triggers-actions.test.sh.
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (owner, _running) = sandbox.capture();
    let owner_id = owner["id"].as_str().unwrap();
    let response = common::request(&sandbox.socket, "POST", "/v1/sessions", "{}");
    let source: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let source_id = source["id"].as_str().unwrap();
    assert!(
        common::request(
            &sandbox.socket,
            "POST",
            &format!("/v1/sessions/{source_id}/done"),
            r#"{"output":"literal {id} $value\n"}"#
        )
        .starts_with("HTTP/1.1 200")
    );
    for (agent, legacy) in [
        ("claude", false),
        ("codex", true),
        ("grok", false),
        ("claude", true),
    ] {
        let suffix = if agent == "grok" {
            " large".repeat(6000)
        } else {
            String::new()
        };
        let prompt = format!(
            "Follow \"quoted\" $literal\nsource {{id}}: {{output}}; $(touch never-execute-ws-prompt){suffix}"
        );
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", owner_id)
            .args(if legacy && agent == "claude" {
                vec!["on", source_id, "done", "--spawn", &prompt]
            } else if legacy {
                vec![
                    "on", source_id, "done", "--spawn", &prompt, "--agent", agent,
                ]
            } else {
                vec!["on", source_id, "done", "--spawn", agent, &prompt]
            })
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let tid = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let (child, _connection) = sandbox.capture();
        let id = child["id"].as_str().unwrap();
        assert_ne!(id, owner_id);
        assert_eq!(child["agent"], agent);
        let argv = child["argv"].as_array().unwrap();
        let expected = format!(
            "Follow \"quoted\" $literal\nsource {source_id}: literal {{id}} $value\n; $(touch never-execute-ws-prompt){suffix}"
        );
        assert!(
            argv.iter().any(|argument| argument == &expected),
            "{argv:?}"
        );
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(session["body"]["spawned"], true);
        assert_eq!(session["body"]["trigger_id"], tid);
        assert_eq!(session["body"]["parent_session_id"], owner_id);
        let window = sandbox.window(id);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .arg("done")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            !sandbox
                .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
                .lines()
                .any(|line| line == window)
        );
        assert!(!fs::read_to_string(sandbox.slots()).unwrap().contains(id));
    }
    let output = sandbox
        .command()
        .current_dir(sandbox.worktree())
        .args(["on", source_id, "done", "--spawn", "codex"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let (child, _running) = sandbox.capture();
    let child_id = child["id"].as_str().unwrap();
    let session = sandbox.json(&["show", child_id, "--json"]);
    let parent = sandbox.json(&[
        "show",
        session["body"]["parent_session_id"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(parent["body"]["human_device_id"], parent["device_id"]);
    assert_eq!(session["workspace_id"], "api/feature");
    assert!(child["argv"].as_array().unwrap().iter().any(|argument| {
        argument
            .as_str()
            .is_some_and(|text| text.contains(source_id) && text.contains("literal {id} $value"))
    }));
    assert!(
        sandbox
            .command()
            .env("WS_SESSION_ID", child_id)
            .arg("done")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!sandbox.worktree().join("never-execute-ws-prompt").exists());
}

#[test]
fn a_closed_pane_cannot_consume_the_reopened_sessions_message() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach",
            ])
            .status
            .success()
    );
    let (initial, _running) = sandbox.capture();
    let id = initial["id"].as_str().unwrap();
    let old_window = sandbox.window(id);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &old_window, "#{pane_id}"]);
    let stop = || {
        common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id": id, "callback_id": ws_core::ids::Ulid::new(),
                "observed_at_us": ws_daemon::hooks::receipt_time_us().unwrap(),
                "state": "stop", "pane": pane, "tmux_socket": sandbox.server,
                "payload": {"hook_event_name":"Stop", "session_id":"retained-conversation"}
            })
            .to_string(),
        )
    };
    assert!(stop().starts_with("HTTP/1.1 201"));
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{id}/stop"),
        "{}",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let sent = sandbox.output(&["send", id, "only for the new window"]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    let (resumed, _resumed) = sandbox.capture();
    assert_eq!(resumed["id"], id);
    let new_window = sandbox.tmux(&[
        "display-message",
        "-p",
        "-t",
        resumed["pane"].as_str().unwrap(),
        "#{window_id}",
    ]);
    assert_ne!(new_window, old_window);
    let response = stop();
    let reply: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(reply.get("decision").is_none(), "{reply}");
    assert!(sandbox.events(id, "hook.delivery").is_empty());
    assert_eq!(sandbox.window(id), new_window);
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
}

#[test]
fn reopening_a_done_agent_keeps_its_live_process_and_delivers_the_message() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    for args in [
        vec![
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "feature",
            "--detach",
        ],
        vec!["add", "api/feature", "--detach"],
    ] {
        let output = sandbox.output(&args);
        assert!(output.status.success(), "{output:?}");
        let (first, mut running) = sandbox.capture();
        let id = first["id"].as_str().unwrap();
        let original_window = sandbox.window(id);
        let done = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .arg("done")
            .output()
            .unwrap();
        assert!(done.status.success(), "{done:?}");
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "finished");
        let sent = sandbox.output(&["send", id, "continue this conversation"]);
        assert_eq!(sent.status.code(), Some(75), "{sent:?}");
        assert_eq!(sandbox.window(id), original_window);
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        assert_eq!(
            sandbox.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        running.write_all(b"d").unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(running.try_clone().unwrap()),
            &mut line,
        )
        .unwrap();
        assert!(line.contains("continue this conversation"), "{line}");
    }
}

#[test]
fn a_dropped_stop_client_receives_one_idle_paste_without_a_second_stop() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feature",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let (capture, mut agent) = sandbox.capture();
    let id = capture["id"].as_str().unwrap();
    let window = sandbox.window(id);
    let pane = capture["pane"].as_str().unwrap();
    let post = |state: &str, payload: Value| {
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "callback_id":ws_core::ids::Ulid::new(), "session_id":id,
                "observed_at_us":ws_daemon::hooks::receipt_time_us().unwrap(),
                "state":state, "pane":pane, "tmux_socket":sandbox.server, "payload":payload
            })
            .to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    };
    post("working", json!({"hook_event_name":"UserPromptSubmit"}));
    agent.write_all(b"d").unwrap();
    sandbox.wait_for(|| {
        sandbox.tmux(&["display-message", "-p", "-t", pane, "#{bracket_paste_flag}"]) == "1"
    });
    assert_eq!(
        sandbox
            .output(&["send", id, "deliver without another stop"])
            .status
            .code(),
        Some(75)
    );
    let callback = json!({
        "callback_id":ws_core::ids::Ulid::new(), "session_id":id,
        "observed_at_us":ws_daemon::hooks::receipt_time_us().unwrap(),
        "state":"stop", "pane":pane, "tmux_socket":sandbox.server,
        "payload":{"hook_event_name":"Stop", "stop_hook_active":false}
    })
    .to_string();
    let mut client = UnixStream::connect(&sandbox.socket).unwrap();
    write!(client, "POST /v1/hooks/ingest HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{callback}", callback.len()).unwrap();
    sandbox.wait_for(|| !sandbox.events(id, "hook.delivery").is_empty());
    drop(client);
    agent
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let mut text = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(agent.try_clone().unwrap()),
        &mut text,
    )
    .unwrap();
    assert_eq!(
        text.matches("deliver without another stop").count(),
        1,
        "{text}"
    );
    sandbox.wait_for(|| !sandbox.events(id, "paste.submitted").is_empty());
    post(
        "working",
        json!({"hook_event_name":"UserPromptSubmit", "turn_id":"pasted-turn"}),
    );
    post("idle", json!({}));
    assert_eq!(
        sandbox.tmux(&[
            "display-message",
            "-p",
            "-t",
            &window,
            "#{@ws_state}|#{@ws_main}"
        ]),
        "idle|idle"
    );
    assert_eq!(sandbox.events(id, "message.delivered").len(), 1);
    assert_eq!(sandbox.events(id, "paste.submitted").len(), 1);
}

#[test]
fn a_message_waits_until_claude_reports_session_start() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feature",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let (capture, mut agent) = sandbox.capture();
    let id = capture["id"].as_str().unwrap();
    let pane = capture["pane"].as_str().unwrap();
    agent.write_all(b"t").unwrap();
    let mut line = String::new();
    let mut reader = std::io::BufReader::new(agent.try_clone().unwrap());
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
    assert_eq!(line, "dialog\n");
    sandbox.wait_for(|| {
        sandbox.tmux(&["display-message", "-p", "-t", pane, "#{bracket_paste_flag}"]) == "1"
    });
    let sent = sandbox.output(&["send", id, "wait for the trust dialog"]);
    assert_eq!(sent.status.code(), Some(75), "{sent:?}");
    // The delivery worker retries every 250 ms. A paste into the dialog would start by now.
    std::thread::sleep(Duration::from_secs(2));
    assert!(sandbox.events(id, "paste.started").is_empty());
    agent.write_all(b"r").unwrap();
    line.clear();
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
    assert!(line.contains("wait for the trust dialog"), "{line}");
    sandbox.wait_for(|| !sandbox.events(id, "paste.submitted").is_empty());
}

#[test]
fn a_message_reopens_a_finished_agent_before_delivery() {
    for agent in ["claude", "codex"] {
        let sandbox = Sandbox::new();
        let repo = sandbox.repo();
        let output = sandbox.output(&[
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "feature",
            "--agent",
            agent,
            "--detach",
        ]);
        assert!(output.status.success(), "{output:?}");
        let (first, mut connection) = sandbox.capture();
        let id = first["id"].as_str().unwrap();
        let old_window = sandbox.window(id);
        let pane = sandbox.tmux(&["display-message", "-p", "-t", &old_window, "#{pane_id}"]);
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/hooks/ingest",
            &json!({
                "session_id": id, "callback_id": ws_core::ids::Ulid::new(),
                "observed_at_us": ws_daemon::hooks::receipt_time_us().unwrap(),
                "state": "stop", "pane": pane, "tmux_socket": sandbox.server,
                "payload": {"hook_event_name":"Stop", "session_id":format!("resume-{agent}")}
            })
            .to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        connection.write_all(b"0").unwrap();
        sandbox.wait_for(|| sandbox.json(&["show", id, "--json"])["state"] == "finished");
        assert!(!fs::read_to_string(sandbox.slots()).unwrap().contains(id));

        let output = if agent == "codex" {
            sandbox.output(&["send", id, "continue the task", "--force"])
        } else {
            sandbox.output(&["send", id, "continue the task"])
        };
        assert_eq!(output.status.code(), Some(75), "{output:?}");
        assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
        let (resumed, mut running) = sandbox.capture();
        assert_eq!(resumed["id"], id);
        assert_ne!(sandbox.window(id), old_window);
        assert!(
            resumed["argv"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|args| args
                    == [
                        json!(if agent == "codex" {
                            "resume"
                        } else {
                            "--resume"
                        }),
                        json!(format!("resume-{agent}")),
                    ])
        );
        assert!(sandbox.events(id, "message.delivered").is_empty());
        let reopened = sandbox.events(id, "session.reopened");
        assert_eq!(reopened.len(), 1);
        let sent = sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|session| sandbox.events(session["id"].as_str().unwrap(), "message.sent"))
            .find(|event| event.body["to_session"] == id)
            .unwrap();
        assert!(sent.ts_ms <= reopened[0].ts_ms);
        running.write_all(b"d").unwrap();
        let mut reader = std::io::BufReader::new(running.try_clone().unwrap());
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        assert_eq!(
            line.trim_end(),
            "[ws #2]\ncontinue the task".escape_debug().to_string()
        );
        assert!(sandbox.events(id, "message.delivered").is_empty());
        running.write_all(b"a").unwrap();
        sandbox.wait_for(|| sandbox.events(id, "message.delivered").len() == 1);
        let delivered = sandbox.events(id, "message.delivered");
        assert_eq!(delivered[0].body["message_id"], sent.event_id.to_string());
        let entries = sandbox.log(id);
        let reopen = entries
            .iter()
            .position(|e| e.event_type == "session.reopened")
            .unwrap();
        let deliver = entries
            .iter()
            .position(|e| e.event_type == "message.delivered")
            .unwrap();
        assert!(reopen < deliver);
    }
}

#[test]
fn workspace_agents_receive_hooks_identity_slots_and_resume_ids() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let mut agents = vec![];
    for (i, agent) in ["claude", "codex", "grok"].into_iter().enumerate() {
        let output = if i == 0 {
            sandbox.output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--agent",
                agent,
                "--detach",
            ])
        } else {
            sandbox.output(&["add", "api/feature", "--agent", agent, "--detach"])
        };
        assert!(output.status.success(), "{output:?}");
        let (capture, _running) = sandbox.capture();
        assert_eq!(capture["agent"], agent);
        assert_eq!(capture["shell"], "ready");
        assert_eq!(capture["legacy_id"], Value::Null);
        assert_eq!(
            capture["cwd"],
            sandbox.worktree().canonicalize().unwrap().to_str().unwrap()
        );
        let sessions = sandbox.json(&["sessions", "--json"]);
        let session = sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == capture["id"])
            .unwrap();
        let slot_file = sandbox.slots();
        let rows = fs::read_to_string(&slot_file).unwrap();
        assert!(rows.contains(&format!(
            "{}\t{agent}\t\t{}\n",
            i + 1,
            session["id"].as_str().unwrap()
        )));
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        if agent == "claude" {
            let settings =
                Path::new(&args[args.iter().position(|v| v == "--settings").unwrap() + 1]);
            let hooks: Value = serde_json::from_slice(&fs::read(settings).unwrap()).unwrap();
            let hook = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap();
            assert_eq!(
                shell_words::split(hook).unwrap(),
                [env!("CARGO_BIN_EXE_ws"), "hook", "stop"]
            );
            assert!(settings.starts_with(sandbox.home.path()));
        } else if agent == "codex" {
            assert!(args.iter().any(|s| s == "--dangerously-bypass-hook-trust"));
            assert!(args.iter().any(|s| s.starts_with("hooks.Stop=")));
        } else {
            assert!(sandbox.home.path().join(".grok/hooks/ws.json").is_file());
        }
        let id = session["id"].as_str().unwrap();
        assert_mcp_registration(&args, agent, id);
        assert_agent_telemetry(&sandbox, &capture, agent, id);
        let rows = rows.replace(
            &format!("\t{agent}\t\t{id}"),
            &format!("\t{agent}\tresume-{agent}\t{id}"),
        );
        fs::write(slot_file, rows).unwrap();
        let window = sandbox.window(id);
        sandbox.tmux(&["kill-window", "-t", &window]);
        let output = sandbox.output(&["open", id, "--detach"]);
        assert!(output.status.success(), "{output:?}");
        let (resumed, running) = sandbox.capture();
        agents.push(running);
        assert_eq!(resumed["id"], id);
        let resumed_args: Vec<String> = serde_json::from_value(resumed["argv"].clone()).unwrap();
        assert_mcp_registration(&resumed_args, agent, id);
        assert_agent_telemetry(&sandbox, &resumed, agent, id);
        let args = resumed["argv"].as_array().unwrap();
        assert!(args.windows(2).any(|p| p
            == [
                json!(if agent == "codex" {
                    "resume"
                } else {
                    "--resume"
                }),
                json!(format!("resume-{agent}"))
            ]));
    }
}

#[test]
fn named_agent_commands_run_in_the_current_tab_and_preserve_failed_slots() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (_initial, _running) = sandbox.capture();
    for (agent, exit_code) in [("claude", 0), ("codex", 7), ("grok", 0)] {
        let window = sandbox.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            "api/feature:",
            "-c",
            sandbox.worktree().to_str().unwrap(),
            "/bin/cat",
        ]);
        let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
        let mut command = sandbox.command();
        command
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent, "Review this", "--model", "test-model"]);
        let child = command.spawn().unwrap();
        let (capture, mut connection) = sandbox.capture();
        assert_eq!(capture["workspace"], "api/feature");
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        assert_eq!(
            &args[args.len() - 3..],
            ["Review this", "--model", "test-model"]
        );
        let id = capture["id"].as_str().unwrap();
        assert_eq!(sandbox.window(id), window);
        let output = sandbox
            .command()
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("this tab already runs a ws session")
        );
        connection
            .write_all(exit_code.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(exit_code), "{output:?}");
        assert_eq!(
            sandbox.tmux(&["show-options", "-wqv", "-t", &window, "@ws_slot"]),
            ""
        );
        let rows = fs::read_to_string(sandbox.slots()).unwrap();
        assert_eq!(rows.contains(id), exit_code != 0);
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(session["exit_code"], exit_code);
        let response = common::request(
            &sandbox.socket,
            "POST",
            &format!("/v1/sessions/{id}/wait"),
            r#"{"for":"exit","timeout":0}"#,
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(
            session["state"],
            if exit_code == 0 { "finished" } else { "failed" }
        );
        sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    }
}

#[test]
fn legacy_agent_and_end_wrappers_run_an_imported_zsh_session() {
    let mut pane = String::new();
    let sandbox = Sandbox::with_setup("", |home, server| {
        let repo = home.join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        let wt = home.join("worktrees/api/feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        let ids = Command::new("tmux")
            .args([
                "-S",
                server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "legacy",
            ])
            .args(["-P", "-F", "#{window_id}\t#{pane_id}", "/bin/cat"])
            .output()
            .unwrap();
        assert!(ids.status.success(), "{ids:?}");
        let ids = String::from_utf8(ids.stdout).unwrap();
        let (window, legacy_pane) = ids.trim_end().split_once('\t').unwrap();
        pane = legacy_pane.into();
        for (key, value) in [("@ws_session", "7"), ("@ws_pane", legacy_pane)] {
            let status = Command::new("tmux")
                .args(["-S", server, "set-option", "-w", "-t", window, key, value])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let legacy = home.join(".ws/sessions/7");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(home.join(".ws/sessions/counter"), "7").unwrap();
        fs::write(
            legacy.join("meta"),
            json!({"agent": "claude", "workspace": "api/feature", "window": window}).to_string(),
        )
        .unwrap();
        fs::write(legacy.join("prompt"), "hello legacy").unwrap();
        let git_dir = git(&wt, &["rev-parse", "--absolute-git-dir"]);
        fs::write(
            Path::new(&git_dir).join("ws-sessions"),
            "1\tclaude\t\t7\n2\tcodex\t\t8\n",
        )
        .unwrap();
    });
    let id = sandbox.json(&["show", "#7", "--json"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let out = sandbox.home.path().join("prompt-out");
    let script = sandbox.home.path().join("legacy-agent.sh");
    fs::write(&script, format!("printf %s \"$2\" > '{}'\n", out.display())).unwrap();
    let launched = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .env("WS_AGENT_SHELL", "/bin/sh -c")
        .args(["_agent", "7", &format!("/bin/sh '{}'", script.display())])
        .output()
        .unwrap();
    assert!(launched.status.success(), "{launched:?}");
    assert_eq!(fs::read_to_string(&out).unwrap(), "hello legacy");
    assert!(!sandbox.home.path().join(".ws/sessions/7/prompt").exists());
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    let wt = sandbox.worktree();
    let ended = sandbox
        .command()
        .args(["_end", wt.to_str().unwrap(), "1", "7", &pane])
        .output()
        .unwrap();
    assert!(ended.status.success(), "{ended:?}");
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    assert_eq!(
        fs::read_to_string(sandbox.slots()).unwrap(),
        "2\tcodex\t\t8\n"
    );
}

#[test]
fn legacy_end_records_the_exit_of_a_zsh_launched_agent() {
    let mut pane = String::new();
    let sandbox = Sandbox::with_setup("", |home, server| {
        let repo = home.join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        let wt = home.join("worktrees/api/feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        let ids = Command::new("tmux")
            .args([
                "-S",
                server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "legacy",
            ])
            .args(["-P", "-F", "#{window_id}\t#{pane_id}", "/bin/cat"])
            .output()
            .unwrap();
        assert!(ids.status.success(), "{ids:?}");
        let ids = String::from_utf8(ids.stdout).unwrap();
        let (window, legacy_pane) = ids.trim_end().split_once('\t').unwrap();
        pane = legacy_pane.into();
        for (key, value) in [("@ws_session", "7"), ("@ws_pane", legacy_pane)] {
            let status = Command::new("tmux")
                .args(["-S", server, "set-option", "-w", "-t", window, key, value])
                .status()
                .unwrap();
            assert!(status.success());
        }
        let legacy = home.join(".ws/sessions/7");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(home.join(".ws/sessions/counter"), "7").unwrap();
        fs::write(
            legacy.join("meta"),
            json!({"agent": "claude", "workspace": "api/feature", "window": window}).to_string(),
        )
        .unwrap();
        let git_dir = git(&wt, &["rev-parse", "--absolute-git-dir"]);
        fs::write(Path::new(&git_dir).join("ws-sessions"), "1\tclaude\t\t7\n").unwrap();
    });
    let id = sandbox.json(&["show", "#7", "--json"])["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let wt = sandbox.worktree();
    let ended = sandbox
        .command()
        .args(["_end", wt.to_str().unwrap(), "1", "7", &pane])
        .output()
        .unwrap();
    assert!(ended.status.success(), "{ended:?}");
    assert_eq!(sandbox.events(&id, "exit").len(), 1);
    assert_eq!(fs::read_to_string(sandbox.slots()).unwrap(), "");
}

#[test]
fn a_clean_exit_releases_its_slot_and_reopening_does_not_steal_another_slot() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (first, mut connection) = sandbox.capture();
    let id = first["id"].as_str().unwrap();
    connection.write_all(b"0").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if sandbox.json(&["show", id, "--json"])["state"] == "finished" {
            break;
        }
        assert!(
            Instant::now() < end,
            "The agent exit did not reach the daemon."
        );
    }
    assert!(
        sandbox
            .output(&["add", "api/feature", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    let (second, _running) = sandbox.capture();
    let second_id = second["id"].as_str().unwrap();
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(
        rows.starts_with(&format!("1\tcodex\t\t{second_id}\n")),
        "{rows}"
    );
    assert!(sandbox.output(&["open", id, "--detach"]).status.success());
    let (reopened, _running) = sandbox.capture();
    assert_eq!(reopened["id"], id);
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(rows.contains(&format!("1\tcodex\t\t{second_id}\n")));
    assert!(rows.contains(&format!("2\tclaude\t\t{id}\n")));
}

#[test]
fn a_manual_tab_can_run_an_agent_after_it_moves_to_another_tmux_session() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    fs::create_dir_all(sandbox.worktree().parent().unwrap()).unwrap();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "feature",
            sandbox.worktree().to_str().unwrap(),
        ],
    );
    assert!(sandbox.output(&["pin", "api/feature"]).status.success());
    let window = sandbox.tmux(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{window_id}",
        "-s",
        "custom",
        "-c",
        sandbox.worktree().to_str().unwrap(),
        "/bin/cat",
    ]);
    sandbox.tmux(&[
        "set-option",
        "-w",
        "-t",
        &window,
        "@ws_path",
        sandbox.worktree().to_str().unwrap(),
    ]);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    let child = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .current_dir(sandbox.worktree())
        .arg("claude")
        .spawn()
        .unwrap();
    let (capture, mut connection) = sandbox.capture();
    assert_eq!(sandbox.window(capture["id"].as_str().unwrap()), window);
    connection.write_all(b"0").unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
}

#[test]
fn split_panes_cannot_prepare_or_finish_the_windows_agent() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (capture, _running) = sandbox.capture();
    let id = capture["id"].as_str().unwrap();
    let window = sandbox.window(id);
    let pane = sandbox.tmux(&[
        "split-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        &window,
        "/bin/cat",
    ]);
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/launch",
        &json!({"id":id, "pane":pane}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 400"), "{result}");
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({"id":id, "pane":pane, "code":0, "user_tab":true}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 200"), "{result}");
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
    assert!(fs::read_to_string(sandbox.slots()).unwrap().contains(id));
}

#[test]
fn interactive_spawn_limits_preserve_slots_and_allow_existing_sessions_to_resume() {
    for (setting, message) in [
        ("max_children", "child limit"),
        ("max_depth", "depth limit"),
    ] {
        let sandbox = Sandbox::with_config(&format!("[limits]\n{setting} = 0\n"));
        let repo = sandbox.repo();
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    "feature",
                    "--detach"
                ])
                .status
                .success()
        );
        let (owner, _running) = sandbox.capture();
        let id = owner["id"].as_str().unwrap();
        let slots = fs::read_to_string(sandbox.slots()).unwrap();
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["add", "api/feature", "--detach"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{output:?}"
        );
        assert_eq!(fs::read_to_string(sandbox.slots()).unwrap(), slots);
        assert_eq!(
            sandbox
                .json(&["sessions", "--json"])
                .as_array()
                .unwrap()
                .len(),
            1
        );
        sandbox.tmux(&["kill-window", "-t", &sandbox.window(id)]);
        let output = sandbox
            .command()
            .env("WS_SESSION_ID", id)
            .args(["open", id, "--detach"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let (resumed, _running) = sandbox.capture();
        assert_eq!(resumed["id"], id);
    }
}

impl Sandbox {
    fn new() -> Self {
        Self::with_config("")
    }
    fn with_config(config: &str) -> Self {
        Self::with_setup(config, |_, _| {})
    }
    /// Runs `setup(home, tmux server)` before the daemon starts and imports legacy state.
    fn with_setup(config: &str, setup: impl FnOnce(&Path, &str)) -> Self {
        Self::with_options(config, setup, false)
    }
    fn with_launch_observation() -> Self {
        Self::with_options("", |_, _| {}, true)
    }
    fn with_options(config: &str, setup: impl FnOnce(&Path, &str), observe_launch: bool) -> Self {
        let home = common::SandboxRoot::new();
        fs::create_dir(home.path().join("state")).unwrap();
        fs::write(home.path().join("state/config.toml"), config).unwrap();
        let socket = home.path().join("wsd.sock");
        let server = home.tmux_socket();
        setup(home.path(), &server);
        let mut command = common::ws(home.path(), &socket);
        if observe_launch {
            command
                .env(
                    "WS_TEST_AGENT_LAUNCH_OBSERVED",
                    home.path().join("launch-requested"),
                )
                .env(
                    "WS_TEST_AGENT_COMMIT_GATE",
                    home.path().join("commit-gate.sock"),
                );
        }
        let listener = UnixListener::bind(home.path().join("agent.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        fs::write(
            home.path().join("agent shell.sh"),
            "export WS_TEST_AGENT_SHELL=ready\nexec /bin/sh -c \"$1\"\n",
        )
        .unwrap();
        command.env(
            "WS_AGENT_SHELL",
            format!(
                "/bin/sh {}",
                shell_words::quote(&home.path().join("agent shell.sh").to_string_lossy())
            ),
        );
        let bin = home.path().join("fake-agents");
        fs::create_dir_all(&bin).unwrap();
        for agent in ["claude", "codex", "grok"] {
            let script = format!(
                r#"#!/usr/bin/python3
import json, os, socket, subprocess, sys, tty
if sys.argv[1:] == ['--version']:
    print({agent:?} + ' ' + ('2.1.302' if {agent:?} == 'claude' else '0.159.2'))
    sys.exit(0)
s = socket.socket(socket.AF_UNIX)
s.connect({:?})
s.sendall((json.dumps(dict(agent={agent:?}, argv=sys.argv[1:], id=os.getenv('WS_SESSION_ID'), legacy_id=os.getenv('WS_SESSION'), cwd=os.getcwd(), workspace=os.getenv('WS_WORKSPACE'), shell=os.getenv('WS_TEST_AGENT_SHELL'), pri=subprocess.run(['ps', '-o', 'pri=', '-p', str(os.getpid())], capture_output=True, text=True).stdout.strip(), pane=os.getenv('TMUX_PANE'), otel=dict(resource=os.getenv('OTEL_RESOURCE_ATTRIBUTES'), endpoint=os.getenv('OTEL_EXPORTER_OTLP_ENDPOINT'), protocol=os.getenv('OTEL_EXPORTER_OTLP_PROTOCOL'), enabled=os.getenv('CLAUDE_CODE_ENABLE_TELEMETRY'), traceparent=os.getenv('TRACEPARENT'), beta=os.getenv('CLAUDE_CODE_ENHANCED_TELEMETRY_BETA'), prompt=os.getenv('OTEL_LOG_USER_PROMPTS'), detail=os.getenv('OTEL_LOG_TOOL_DETAILS'), content=os.getenv('OTEL_LOG_TOOL_CONTENT'))))+'\n').encode())
control = s.recv(1)
if control in (b'd', b't'):
    tty.setraw(0)
    os.write(1, b'\x1b[?2004h')
    if control == b't':
        # Claude shows its trust dialog before SessionStart. The dialog also accepts pastes.
        s.sendall(b'dialog\n')
        s.recv(1)
    if {agent:?} == 'claude':
        subprocess.run([os.environ['WS_BIN'], 'hook', 'ready'], input=json.dumps(dict(hook_event_name='SessionStart', source='startup')).encode(), check=True)
    data = b''
    while not data.endswith(b'\r'):
        data += os.read(0, 4096)
    text = data.removeprefix(b'\x1b[200~').removesuffix(b'\r').removesuffix(b'\x1b[201~').decode()
    s.sendall((text.encode('unicode_escape').decode()+'\n').encode())
    if s.recv(1) == b'a':
        subprocess.run([os.environ['WS_BIN'], 'hook', 'working'], input=json.dumps(dict(hook_event_name='UserPromptSubmit', turn_id='next-turn', session_id='resume-{agent}')).encode(), check=True)
    s.recv(1)
    sys.exit(0)
sys.exit(int(control or b'0'))
"#,
                home.path().join("agent.sock").to_str().unwrap()
            );
            fs::write(bin.join(agent), script).unwrap();
            fs::set_permissions(bin.join(agent), fs::Permissions::from_mode(0o755)).unwrap();
        }
        command
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("WS_ROOT", home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        let daemon = common::Daemon::start(command, &socket);
        assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
        let sandbox = Self {
            home,
            socket,
            server,
            listener,
            _daemon: daemon,
        };
        sandbox.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keepalive",
            "/bin/cat",
        ]);
        sandbox
    }
    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env(
                "WS_AGENT_SHELL",
                format!(
                    "/bin/sh {}",
                    shell_words::quote(&self.home.path().join("agent shell.sh").to_string_lossy())
                ),
            )
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        command
    }
    fn output(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let out = self.output(args);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-S", &self.server])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim_end().into()
    }
    fn capture(&self) -> (Value, UnixStream) {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(30)))
                        .unwrap();
                    let mut bytes = vec![];
                    loop {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        if byte[0] == b'\n' {
                            break;
                        }
                        bytes.push(byte[0]);
                    }
                    return (serde_json::from_slice(&bytes).unwrap(), stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "The fake agent did not start.");
                    std::thread::yield_now();
                }
                Err(e) => panic!("{e}"),
            }
        }
    }
    fn wait_for(&self, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "The expected event did not occur."
            );
            std::thread::yield_now();
        }
    }
    fn log(&self, id: &str) -> Vec<ws_core::log::LogEntry> {
        ws_core::log::SessionLog::new(
            &self.home.path().join("state"),
            id.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries
    }
    fn events(&self, id: &str, event: &str) -> Vec<ws_core::log::LogEntry> {
        self.log(id)
            .into_iter()
            .filter(|entry| entry.event_type == event)
            .collect()
    }
    fn repo(&self) -> PathBuf {
        let repo = self.home.path().join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        repo
    }
    fn worktree(&self) -> PathBuf {
        self.home.path().join("worktrees/api/feature")
    }
    fn slots(&self) -> PathBuf {
        PathBuf::from(git(&self.worktree(), &["rev-parse", "--absolute-git-dir"]))
            .join("ws-sessions")
    }
    fn window(&self, id: &str) -> String {
        self.tmux(&["list-windows", "-a", "-F", "#{window_id}\t#{@ws_session}"])
            .lines()
            .find_map(|l| {
                let (w, s) = l.split_once('\t')?;
                (s == id).then(|| w.into())
            })
            .unwrap()
    }
}
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap().trim_end().into()
}

fn assert_mcp_registration(args: &[String], agent: &str, id: &str) {
    if agent == "claude" {
        let index = args.iter().position(|arg| arg == "--mcp-config").unwrap();
        let config: Value = serde_json::from_slice(&fs::read(&args[index + 1]).unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["ws"]["command"],
            env!("CARGO_BIN_EXE_ws")
        );
        assert_eq!(config["mcpServers"]["ws"]["args"], json!(["mcp"]));
        assert_eq!(config["mcpServers"]["ws"]["env"]["WS_SESSION_ID"], id);
    } else if agent == "codex" {
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", "mcp_servers.ws.args=[\"mcp\"]"])
        );
        assert!(
            args.iter()
                .any(|arg| arg == &format!("mcp_servers.ws.env.WS_SESSION_ID=\"{id}\""))
        );
    }
}

fn assert_agent_telemetry(sandbox: &Sandbox, capture: &Value, agent: &str, id: &str) {
    use ws_core::{device::Device, semconv};
    if agent == "grok" {
        return;
    }
    let state = sandbox.home.path().join("state");
    let device = Device::load(&state).unwrap();
    let resources: std::collections::BTreeMap<_, _> = capture["otel"]["resource"]
        .as_str()
        .unwrap()
        .split(',')
        .map(|part| part.split_once('=').unwrap())
        .collect();
    assert_eq!(resources[semconv::SESSION_ID], id);
    assert_eq!(resources[semconv::PARENT_SESSION_ID], "");
    assert_eq!(resources[semconv::DEVICE_ID], device.device_id.to_string());
    assert_eq!(resources[semconv::PROJECT], "api/feature");
    for key in ["traceparent", "beta", "prompt", "detail", "content"] {
        assert!(capture["otel"][key].is_null(), "{key}");
    }
    let endpoint = fs::read_to_string(state.join("otlp.endpoint")).unwrap();
    if agent == "claude" {
        assert_eq!(capture["otel"]["endpoint"], endpoint.trim());
        assert_eq!(capture["otel"]["protocol"], "http/json");
        assert_eq!(capture["otel"]["enabled"], "1");
    } else {
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        assert!(
            args.iter()
                .any(|arg| arg.contains(&format!("{}/v1/logs", endpoint.trim())))
        );
        assert!(args.iter().any(|arg| arg == "otel.trace_exporter=\"none\""));
    }
}
