#[allow(dead_code)]
mod common;

use std::{fs, path::PathBuf};

use serde_json::{Value, json};

struct Sandbox {
    _daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
    loop_dir: PathBuf,
}

#[test]
fn skills_tier_changes_the_plan_and_refuses_a_review_that_started() {
    let sandbox = Sandbox::new();
    let plan = sandbox.loop_dir.join("plan.md");
    fs::write(&plan, "# Plan\n## Phases\n### Phase 9 — Review `[kind: pr-review]` `[status: todo]` `[review: standard]` `[rounds: 2]`\n").unwrap();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    let id = owner["id"].as_str().unwrap();
    let run = || {
        common::ws(sandbox.home.path(), &sandbox.socket)
            .args(["loop", "tier", id, "9", "max", "--rounds", "4", "--json"])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(result.status.success(), "{result:?}");
    let entry: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(entry["type"], "loop.plan");
    let text = fs::read_to_string(&plan).unwrap();
    assert!(text.contains("[review: max]"));
    assert!(text.contains("[rounds: 4]"));
    assert_eq!(entry["body"]["text"], text);
    assert_eq!(entry["body"]["sha"].as_str().unwrap().len(), 64);
    fs::write(
        sandbox.loop_dir.join("state.json"),
        r#"{"runId":"loop-test","phases":{"9":{"status":"running","attempt":1}}}"#,
    )
    .unwrap();
    let result = run();
    assert!(!result.status.success(), "{result:?}");
    assert_eq!(fs::read_to_string(&plan).unwrap(), text);
    fs::write(
        sandbox.loop_dir.join("state.json"),
        r#"{"runId":"loop-test","phases":{}}"#,
    )
    .unwrap();
    sandbox.request(
        &format!("/v1/sessions/{id}/events"),
        json!({"type":"loop.review", "body":{
        "phase":9,"repo":"none","key":"r9","role":"review","round":1,
        "state":"started","session":"reviewer","provider":"gpt"}}),
    );
    let result = run();
    assert!(!result.status.success(), "{result:?}");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("already started"),
        "{result:?}"
    );
    assert_eq!(fs::read_to_string(plan).unwrap(), text);
}

#[test]
fn skills_cli_controls_record_events_and_bridge_runner_files() {
    let sandbox = Sandbox::new();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    let id = owner["id"].as_str().unwrap();
    let run = |args: &[&str]| {
        let output = common::ws(sandbox.home.path(), &sandbox.socket)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let paused = run(&["loop", "pause", id, "7", "--json"]);
    assert_eq!(paused["body"]["action"], "pause");
    assert!(sandbox.loop_dir.join("control/pause-7").exists());
    run(&["loop", "model", id, "7", "claude:claude-opus-5-5", "--json"]);
    let note = run(&["loop", "note", id, "7", "Use the public API.", "--json"]);
    assert_eq!(note["body"]["text"], "Use the public API.");
    run(&["loop", "resume", id, "7", "--json"]);
    assert!(!sandbox.loop_dir.join("control/pause-7").exists());
    run(&["loop", "pause", id, "--json"]);
    assert!(sandbox.loop_dir.join("control/pause").exists());
    run(&["loop", "resume", id, "--json"]);
    assert!(!sandbox.loop_dir.join("control/pause").exists());
}

#[test]
fn stop_writes_the_dashboard_pause_control_and_audits_the_caller() {
    let sandbox = Sandbox::new();
    let parent = sandbox.request("/v1/sessions", json!({}));
    let result = sandbox.request(
        "/v1/loops/stop",
        json!({
            "loop_dir":sandbox.loop_dir, "parent_session_id":parent["id"]
        }),
    );
    assert_eq!(result["run_id"], "loop-test");
    assert_eq!(result["paused"], true);
    let pause = fs::read_to_string(sandbox.loop_dir.join("control/pause")).unwrap();
    assert!(!pause.trim().is_empty());
    assert_eq!(
        fs::read_to_string(sandbox.loop_dir.join("state.json")).unwrap(),
        "{\"runId\":\"loop-test\",\"phases\":{}}"
    );
    let entries = ws_core::log::SessionLog::new(
        &sandbox.home.path().join("state"),
        parent["id"].as_str().unwrap().parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    )
    .read()
    .unwrap()
    .entries;
    assert!(entries.iter().any(|entry| entry.event_type == "api.called"
        && entry.body["operation"] == "loop.stop"
        && entry.body["parent_session_id"] == parent["id"]));
}

#[test]
fn start_runs_the_existing_orchestrator_as_a_detached_session() {
    let sandbox = Sandbox::new();
    let parent = sandbox.request("/v1/sessions", json!({}));
    let child = sandbox.request(
        "/v1/loops/start",
        json!({
            "loop_dir":sandbox.loop_dir, "parent_session_id":parent["id"]
        }),
    );
    let id = child["id"].as_str().unwrap();
    sandbox.wait_file("engine-ready");
    assert_eq!(child["body"]["kind"], "headless");
    assert_eq!(child["parent_session_id"], parent["id"]);
    let state: Value =
        serde_json::from_slice(&fs::read(sandbox.loop_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["wsSession"], id);
    assert_eq!(state["phases"], json!({}));
    assert_eq!(
        fs::read_to_string(sandbox.loop_dir.join("sub/received-seed")).unwrap(),
        "Synthetic seed\n"
    );
    assert_eq!(
        fs::read_to_string(sandbox.loop_dir.join("sub/engine-session")).unwrap(),
        id
    );
    assert!(
        fs::read_to_string(sandbox.loop_dir.join("sub/orch.pid"))
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
            > 1
    );
    let duplicate = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":sandbox.loop_dir}).to_string(),
    );
    assert!(duplicate.starts_with("HTTP/1.1 400"), "{duplicate}");
    fs::write(sandbox.loop_dir.join("sub/release"), "release\n").unwrap();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args(["wait", id, "--for", "exit", "--timeout", "30", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(sandbox.loop_dir.join("sub/PHASES_DONE").exists());
    let entries = ws_core::log::SessionLog::new(
        &sandbox.home.path().join("state"),
        parent["id"].as_str().unwrap().parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    )
    .read()
    .unwrap()
    .entries;
    assert!(
        entries
            .iter()
            .any(|entry| entry.body["operation"] == "loop.start")
    );
}

#[test]
fn cli_create_makes_a_loop_session_without_a_process_and_events_record_their_source() {
    let sandbox = Sandbox::new();
    let ws = |args: &[&str]| {
        let output = common::ws(sandbox.home.path(), &sandbox.socket)
            .args(args)
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let dir = sandbox.loop_dir.to_str().unwrap();
    let created = ws(&["loop", "create", "--dir", dir, "--json"]);
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["body"]["kind"], "loop");
    assert_eq!(ws(&["loop", "create", "--dir", dir, "--json"])["id"], id);
    let tool = ws(&[
        "event",
        "loop.note",
        "--target",
        id,
        "--source",
        "tool",
        "--body",
        r#"{"text":"t"}"#,
    ]);
    assert_eq!(tool["body"]["source"], "tool");
    let agent = ws(&[
        "event",
        "loop.note",
        "--target",
        id,
        "--body",
        r#"{"text":"a"}"#,
    ]);
    assert_eq!(agent["body"]["source"], "agent");
    let rejected = common::ws(sandbox.home.path(), &sandbox.socket)
        .args(["event", "loop.phase", "--target", id, "--body", "{}"])
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("loop.phase"));
    let sessions = ws(&["sessions", "--json"]);
    let session = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == id)
        .unwrap();
    assert_eq!(session["body"]["kind"], "loop");
    let log = ws(&["log", id, "--type", "session.running"]);
    assert!(log["entries"].as_array().unwrap().is_empty());
}

#[test]
fn cli_start_and_stop_use_the_loop_api() {
    let sandbox = Sandbox::new();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args([
            "loop",
            "start",
            sandbox.loop_dir.to_str().unwrap(),
            "--json",
        ])
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let child: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(child["body"]["kind"], "headless");
    sandbox.wait_file("engine-ready");
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .current_dir(sandbox.loop_dir.parent().unwrap())
        .args(["loop", "stop", ".loop", "--json"])
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let paused: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(paused["paused"], true);
    assert!(sandbox.loop_dir.join("control/pause").exists());
}

#[test]
fn top_prefers_the_orchestrator_session_over_the_child_worktree() {
    let sandbox = Sandbox::new();
    sandbox.dashboard();
    let parent = sandbox.request("/v1/sessions", json!({}));
    let other = sandbox.home.path().join("other");
    fs::create_dir_all(other.join(".loop")).unwrap();
    fs::write(other.join(".loop/state.json"), r#"{"runId":"wrong-loop"}"#).unwrap();
    let child = sandbox.request("/v1/sessions", json!({"headless":true,"command":["/usr/bin/true"],"cwd":other,"parent_session_id":parent["id"]}));
    fs::write(
        sandbox.loop_dir.join("state.json"),
        json!({"runId":"loop-test","wsSession":parent["id"]}).to_string(),
    )
    .unwrap();
    let store = sandbox.home.path().join(".loop/loops");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("loop-test.json"),
        json!({"loopDir":sandbox.loop_dir}).to_string(),
    )
    .unwrap();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .current_dir(&other)
        .args(["top", "--once", "--graph"])
        .env("WS_SESSION_ID", child["id"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["args"], json!(["loop-test", "--once", "--graph"]));
    assert_eq!(
        result["cwd"],
        sandbox
            .loop_dir
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
}

#[test]
fn top_uses_the_session_worktree_and_preserves_the_dashboard_exit_code() {
    let sandbox = Sandbox::new();
    sandbox.dashboard();
    let worktree = sandbox.home.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    let session = sandbox.request(
        "/v1/sessions",
        json!({"headless":true,"command":["/usr/bin/true"],"cwd":worktree}),
    );
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .current_dir(sandbox.home.path())
        .args(["top", "--once", "--worktrees"])
        .env("WS_SESSION_ID", session["id"].as_str().unwrap())
        .env("DASHBOARD_EXIT", "7")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["args"], json!(["--once", "--worktrees"]));
    assert_eq!(
        result["cwd"],
        worktree.canonicalize().unwrap().to_str().unwrap()
    );
}

#[test]
fn start_uses_the_orchestrator_default_prompt_when_no_seed_exists() {
    let sandbox = Sandbox::new();
    fs::remove_file(sandbox.loop_dir.join("sub/prompt-seed.md")).unwrap();
    let child = sandbox.request("/v1/loops/start", json!({"loop_dir":sandbox.loop_dir}));
    assert_eq!(child["body"]["kind"], "headless");
    sandbox.wait_file("engine-ready");
    let prompt = fs::read_to_string(sandbox.loop_dir.join("sub/received-seed")).unwrap();
    assert!(prompt.contains("headless sub-orchestrator"), "{prompt}");
    assert!(prompt.contains("loop-execute resume"), "{prompt}");
}

#[test]
fn start_refuses_an_orchestrator_registered_by_the_shell_entry_point() {
    let sandbox = Sandbox::new();
    let existing = sandbox.request(
        "/v1/sessions",
        json!({
            "headless":true,"command":["/bin/sh","-c","read -r release < .loop/sub/release"],
            "cwd":sandbox.loop_dir.parent().unwrap()
        }),
    );
    fs::write(
        sandbox.loop_dir.join("state.json"),
        json!({"runId":"loop-test","wsSession":existing["id"]}).to_string(),
    )
    .unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":sandbox.loop_dir}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("running orchestrator"), "{response}");
    assert!(!sandbox.loop_dir.join("sub/engine-ready").exists());
}

#[tokio::test]
async fn mcp_start_runs_the_same_headless_orchestrator() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
    let sandbox = Sandbox::new();
    let parent = sandbox.request("/v1/sessions", json!({}));
    let mut command =
        tokio::process::Command::from(common::ws(sandbox.home.path(), &sandbox.socket));
    command
        .arg("mcp")
        .env("WS_SESSION_ID", parent["id"].as_str().unwrap());
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client.call_tool(
            CallToolRequestParams::new("loop_start").with_arguments(
                json!({"loop_dir":sandbox.loop_dir})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let child = result.structured_content.unwrap();
    assert_eq!(child["parent_session_id"], parent["id"]);
    assert_eq!(child["body"]["kind"], "headless");
    sandbox.wait_file("engine-ready");
    client.cancel().await.unwrap();
}

#[test]
fn loop_controls_reject_missing_state_and_a_parked_milestone() {
    let sandbox = Sandbox::new();
    for body in [
        json!({"loop_dir":"relative"}),
        json!({"loop_dir":sandbox.home.path()}),
        json!({"loop_dir":sandbox.loop_dir,"unknown":true}),
    ] {
        for operation in ["start", "stop"] {
            let response = common::request(
                &sandbox.socket,
                "POST",
                &format!("/v1/loops/{operation}"),
                &body.to_string(),
            );
            assert!(!response.starts_with("HTTP/1.1 2"), "{response}");
        }
    }
    assert!(!sandbox.loop_dir.join("control/pause").exists());
    fs::write(
        sandbox.loop_dir.join("state.json"),
        r#"{"runId":"loop-test","awaitingMerge":{"milestone":2}}"#,
    )
    .unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":sandbox.loop_dir}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("milestone merge"), "{response}");
    assert!(!sandbox.loop_dir.join("sub/engine-ready").exists());
}

#[test]
fn a_started_child_marks_its_phase_running_and_an_open_merge_gate_refuses_the_next_milestone() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("gate-loop");
    fs::create_dir_all(&directory).unwrap();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":directory}));
    let id = owner["id"].as_str().unwrap();
    let phase = |n: u64, milestone: u64| {
        json!({"n":n,"title":format!("P{n}"),"kind":"work","output":"pr","milestone":milestone,
            "gate":null,"repo":null,"branch":null,"depends_on":[],"status":"todo"})
    };
    let event = |kind: &str, body: Value| {
        sandbox.request(
            &format!("/v1/sessions/{id}/events"),
            json!({"type":kind,"body":body}),
        )
    };
    event("loop.phases", json!({"phases":[phase(1, 1), phase(2, 2)]}));
    let spawn = |n: u64| {
        common::request(
            &sandbox.socket,
            "POST",
            "/v1/sessions",
            &json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.home.path(),
                "key":format!("p{n}"),"loop":{"loop":id,"phase":n,"attempt":1}})
            .to_string(),
        )
    };
    assert!(spawn(1).starts_with("HTTP/1.1 201"));
    let state = |sandbox: &Sandbox| {
        let response = common::get(&sandbox.socket, &format!("/v1/loops/{id}"));
        serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    assert_eq!(state(&sandbox)["phases"]["1"]["status"], "running");
    event(
        "loop.gate",
        json!({"kind":"merge","state":"open","milestone":1,"prs":[]}),
    );
    assert_eq!(state(&sandbox)["status"], "awaiting-merge");
    let refused = spawn(2);
    assert!(refused.starts_with("HTTP/1.1 409"), "{refused}");
    assert!(refused.contains("milestone_gate_open"), "{refused}");
    event(
        "loop.gate",
        json!({"kind":"merge","state":"merged","milestone":1,"prs":[]}),
    );
    assert!(spawn(2).starts_with("HTTP/1.1 201"));
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("ws.sock");
        let loop_dir = home.path().join("coordinator/.loop");
        fs::create_dir_all(&loop_dir).unwrap();
        fs::write(
            loop_dir.join("state.json"),
            "{\"runId\":\"loop-test\",\"phases\":{}}",
        )
        .unwrap();
        let scripts = home.path().join("dotfile");
        fs::create_dir_all(&scripts).unwrap();
        std::os::unix::fs::symlink(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../loop-orchestrator.sh")
                .canonicalize()
                .unwrap(),
            scripts.join("loop-orchestrator.sh"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../loop-plan-tier.mjs")
                .canonicalize()
                .unwrap(),
            scripts.join("loop-plan-tier.mjs"),
        )
        .unwrap();
        let sub = loop_dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("prompt-seed.md"), "Synthetic seed\n").unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(sub.join("release"))
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            scripts.join("emit.sh"),
            "loop_emit() { cat >/dev/null; }\nloop_ensure_daemon() { :; }\n",
        )
        .unwrap();
        let engine = scripts.join("engine");
        fs::write(
            &engine,
            r#"#!/bin/sh
cat > "$LOOP_SUB_DIR/received-seed"
printf '%s' "$WS_SESSION_ID" > "$LOOP_SUB_DIR/engine-session"
printf ready > "$LOOP_SUB_DIR/engine-ready"
read -r release < "$LOOP_SUB_DIR/release"
printf '{"outcome":"complete"}' > "$LOOP_SUB_DIR/status.json"
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = common::ws(home.path(), &socket);
        command
            .env("ENGINE_CMD", engine)
            .env("LOOP_EMIT_SH", scripts.join("emit.sh"))
            .env("LOOP_NOTIFY_SH", "/usr/bin/true")
            .env("LOOP_ORCH_POLL_SEC", "1")
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            _daemon: daemon,
            home,
            socket,
            loop_dir,
        }
    }

    fn dashboard(&self) {
        fs::write(self.home.path().join("dotfile/loop-top.mjs"),
            "console.log(JSON.stringify({args:process.argv.slice(2),cwd:process.cwd()})); process.exit(Number(process.env.DASHBOARD_EXIT || 0));").unwrap();
    }

    fn wait_file(&self, name: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !self.loop_dir.join("sub").join(name).exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "The engine did not write {name}."
            );
            std::thread::yield_now();
        }
    }

    fn request(&self, path: &str, body: Value) -> Value {
        let response = common::request(&self.socket, "POST", path, &body.to_string());
        assert!(response.starts_with("HTTP/1.1 2"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let response = common::get(&self.socket, "/v1/sessions");
        if let Some((_, body)) = response.split_once("\r\n\r\n")
            && let Ok(sessions) = serde_json::from_str::<Vec<Value>>(body)
        {
            for session in sessions {
                if session["body"]["kind"] == "headless" {
                    let _ = common::request(
                        &self.socket,
                        "POST",
                        &format!("/v1/sessions/{}/stop", session["id"].as_str().unwrap()),
                        "{}",
                    );
                }
            }
        }
    }
}
