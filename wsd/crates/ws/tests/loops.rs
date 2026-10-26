#[allow(dead_code)]
mod common;

use std::{fs, path::PathBuf};

use serde_json::{Value, json};
use sha2::Digest;

struct Sandbox {
    _daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
    loop_dir: PathBuf,
}

#[test]
fn loop_plan_identity_survives_restart() {
    let mut sandbox = Sandbox::new();
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: durable-plan\n-->\n# Plan\n",
    )
    .unwrap();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    assert_eq!(owner["body"]["plan_id"], "durable-plan");
    let id = owner["id"].as_str().unwrap();
    let again = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    assert_eq!(again["id"], owner["id"]);
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    let mut command = common::ws(sandbox.home.path(), &sandbox.socket);
    command
        .env_remove("WS_LOOP_ID")
        .env_remove("LOOP_DAEMON_URL");
    sandbox._daemon = common::Daemon::spawn(command);
    sandbox
        ._daemon
        .wait_ready_for(&sandbox.socket, std::time::Duration::from_secs(30));
    let again = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    assert_eq!(again["id"], owner["id"]);
    for target in [
        id.to_owned(),
        format!("#{}", owner["alias"]),
        "durable-plan".into(),
    ] {
        let output = common::ws(sandbox.home.path(), &sandbox.socket)
            .args(["loop", "get", &target, "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let state: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(state["id"], owner["id"]);
        assert_eq!(state["plan_id"], "durable-plan");
    }
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args(["loop", "list", "--json"])
        .output()
        .unwrap();
    let loops: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(loops.as_array().unwrap().len(), 1);
    assert_eq!(loops[0]["plan_id"], "durable-plan");
}

#[test]
fn loop_create_requires_a_header_id_and_accepts_an_explicit_override() {
    let sandbox = Sandbox::new();
    for text in [
        "# Plan\n",
        "# Plan\n<!-- loop-plan\nplanId: outside-header\n-->\n",
        "<!-- loop-plan\nplanId:  \n-->\n",
    ] {
        fs::write(sandbox.loop_dir.join("plan.md"), text).unwrap();
        let response = common::request(
            &sandbox.socket,
            "POST",
            "/v1/loops/create",
            &json!({"loop_dir":sandbox.loop_dir}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    }
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: header-plan\n-->\n",
    )
    .unwrap();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args([
            "loop",
            "create",
            "--dir",
            sandbox.loop_dir.to_str().unwrap(),
            "--plan-id",
            "explicit-plan",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let owner: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(owner["body"]["plan_id"], "explicit-plan");
    let other = sandbox.home.path().join("other-loop");
    fs::create_dir(&other).unwrap();
    std::thread::scope(|scope| {
        let creates: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    sandbox.request(
                        "/v1/loops/create",
                        json!({"loop_dir":other,"plan_id":"explicit-plan"}),
                    )
                })
            })
            .collect();
        for create in creates {
            assert_eq!(create.join().unwrap()["id"], owner["id"]);
        }
    });
    let response = common::get(&sandbox.socket, "/v1/loops");
    let loops: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(loops.as_array().unwrap().len(), 1);
    let paused = sandbox.request("/v1/loops/explicit-plan/pause", json!({}));
    assert_eq!(paused["session_id"], owner["id"]);
    let response = common::get(&sandbox.socket, "/v1/loops/explicit-plan/check");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

#[test]
fn existing_plan_identity_takes_precedence_over_the_directory() {
    let sandbox = Sandbox::new();
    let first = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":sandbox.loop_dir,"plan_id":"first-plan"}),
    );
    let other = sandbox.home.path().join("second-loop");
    fs::create_dir(&other).unwrap();
    let second = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":other,"plan_id":"second-plan"}),
    );
    assert_ne!(first["id"], second["id"]);
    // Either registry ordering must choose the plan match over a directory match.
    for (directory, plan_id, expected) in [
        (&other, "first-plan", &first["id"]),
        (&sandbox.loop_dir, "second-plan", &second["id"]),
    ] {
        let again = sandbox.request(
            "/v1/loops/create",
            json!({"loop_dir":directory,"plan_id":plan_id}),
        );
        assert_eq!(&again["id"], expected);
    }
}

#[test]
fn legacy_loop_plan_event_preserves_children_and_history() {
    let mut sandbox = Sandbox::new();
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: file-plan\n-->\n",
    )
    .unwrap();
    let id = sandbox.seed_legacy_loop(Some("<!-- loop-plan\nplanId: event-plan\n-->\n"));
    let child = sandbox.request("/v1/sessions", json!({"parent_session_id":id}));
    sandbox.request(
        &format!("/v1/sessions/{id}/events"),
        json!({"type":"loop.note","body":{"text":"Keep the existing history."}}),
    );
    let response = common::get(&sandbox.socket, "/v1/loops/event-plan");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let state: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(state["id"], id);
    assert_eq!(state["plan_id"], "event-plan");
    assert_eq!(state["notes"][0]["text"], "Keep the existing history.");
    let other = sandbox.home.path().join("moved-loop");
    fs::create_dir(&other).unwrap();
    let owner = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":other,"plan_id":"event-plan"}),
    );
    assert_eq!(owner["id"], id);
    let response = common::get(
        &sandbox.socket,
        &format!("/v1/sessions/{}", child["id"].as_str().unwrap()),
    );
    let fetched: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(fetched["parent_session_id"], id);
    let response = common::get(&sandbox.socket, &format!("/v1/sessions/{id}/log"));
    let log: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["type"] == "session.created")
            .count(),
        1
    );
}

#[test]
fn legacy_loop_uses_the_file_header_and_canonical_directory() {
    let mut sandbox = Sandbox::new();
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: file-plan\n-->\n",
    )
    .unwrap();
    let id = sandbox.seed_legacy_loop(None);
    let response = common::get(&sandbox.socket, "/v1/loops/file-plan");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let link = sandbox.home.path().join("linked-loop");
    std::os::unix::fs::symlink(&sandbox.loop_dir, &link).unwrap();
    let owner = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":link,"plan_id":"new-header-plan"}),
    );
    assert_eq!(owner["id"], id);
    let response = common::get(&sandbox.socket, "/v1/loops");
    let loops: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(loops.as_array().unwrap().len(), 1);
    assert_eq!(loops[0]["plan_id"], "file-plan");
}

#[test]
fn loop_ack_cli_records_the_human_reason_and_refuses_an_agent() {
    let sandbox = Sandbox::new();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let id = owner["id"].as_str().unwrap();
    sandbox.request(
        &format!("/v1/sessions/{id}/events"),
        json!({"type":"loop.phase",
        "source":"tool","body":{"n":1,"status":"merged","session":null}}),
    );
    let response = common::get(&sandbox.socket, &format!("/v1/loops/{id}"));
    let state: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let violation = state["violations"][0]["id"].as_str().unwrap();
    let agent = sandbox.request("/v1/sessions", json!({}));
    for identity in ["WS_SESSION_ID", "WS_SESSION"] {
        let denied = common::ws(sandbox.home.path(), &sandbox.socket)
            .env(identity, agent["id"].as_str().unwrap())
            .args([
                "loop",
                "ack",
                violation,
                "--reason",
                "I checked the merge.",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(!denied.status.success());
        assert!(
            String::from_utf8_lossy(&denied.stderr).contains("Only the human"),
            "{denied:?}"
        );
    }
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args([
            "loop",
            "ack",
            violation,
            "--reason",
            "I checked the merge.",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let entry: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(entry["type"], "loop.control");
    assert_eq!(entry["body"]["violation"], violation);
    assert_eq!(entry["body"]["reason"], "I checked the merge.");
}

#[tokio::test]
async fn loop_ack_mcp_uses_the_caller_identity() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
    let sandbox = Sandbox::new();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let id = owner["id"].as_str().unwrap();
    sandbox.request(
        &format!("/v1/sessions/{id}/events"),
        json!({"type":"loop.phase",
        "source":"tool","body":{"n":1,"status":"merged","session":null}}),
    );
    let response = common::get(&sandbox.socket, &format!("/v1/loops/{id}"));
    let state: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let violation = state["violations"][0]["id"].as_str().unwrap();
    let agent = sandbox.request("/v1/sessions", json!({}));
    for human in [false, true] {
        let mut command =
            tokio::process::Command::from(common::ws(sandbox.home.path(), &sandbox.socket));
        command.arg("mcp");
        if !human {
            command.env("WS_SESSION_ID", agent["id"].as_str().unwrap());
        }
        let client = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            ().serve(TokioChildProcess::new(command).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        let tools = client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|tool| tool.name == "loop_ack"));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.call_tool(
                CallToolRequestParams::new("loop_ack").with_arguments(
                    json!({"id":violation,"reason":"I checked the merge."})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        if human {
            assert_ne!(result.is_error, Some(true), "{result:?}");
            assert_eq!(
                result.structured_content.unwrap()["body"]["violation"],
                violation
            );
        } else {
            assert_eq!(result.is_error, Some(true), "{result:?}");
            assert!(
                result.structured_content.unwrap()["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Only the human")
            );
        }
        client.cancel().await.unwrap();
    }
}

#[test]
fn loop_enforcement_sends_one_notification_for_a_new_hard_result() {
    let sandbox = Sandbox::with_notifications(true);
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let id = owner["id"].as_str().unwrap();
    let event = |kind: &str, body: Value| {
        sandbox.request(
            &format!("/v1/sessions/{id}/events"),
            json!({"type":kind,"source":"tool","body":body}),
        )
    };
    event(
        "loop.guard",
        json!({"session":"child","rule":"write","command":"git push"}),
    );
    event(
        "loop.phase",
        json!({"n":1,"status":"merged","session":null}),
    );
    for _ in 0..3 {
        event("loop.note", json!({"text":"Check the existing result."}));
        let response = common::request(&sandbox.socket,"POST","/v1/sessions",
            &json!({"headless":true,"command":["/bin/true"],"cwd":sandbox.home.path(),"key":"blocked",
                "loop":{"loop":id,"phase":2,"attempt":1}}).to_string());
        assert!(response.starts_with("HTTP/1.1 409"), "{response}");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let notifications = loop {
        let text =
            fs::read_to_string(sandbox.home.path().join("notifications.jsonl")).unwrap_or_default();
        if text.ends_with('\n') {
            break text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "The adapter did not receive a notification."
        );
        std::thread::yield_now();
    };
    assert_eq!(notifications.lines().count(), 1);
    let event: Value = serde_json::from_str(notifications.trim()).unwrap();
    assert_eq!(event["loop"], id);
    assert_eq!(event["state"], "waiting");
    let response = common::get(&sandbox.socket, &format!("/v1/loops/{id}"));
    let state: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(state["violations"].as_array().unwrap().len(), 2);
    assert_eq!(state["violations"][1]["id"], event["violation"]);
}

#[test]
fn loop_plan_cli_prints_the_text_writes_a_file_and_exits_3_without_a_plan() {
    let sandbox = Sandbox::new();
    let ws = |args: &[&str]| {
        common::ws(sandbox.home.path(), &sandbox.socket)
            .args(args)
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .output()
            .unwrap()
    };
    let dir = sandbox.loop_dir.to_str().unwrap();
    let created = ws(&[
        "loop",
        "create",
        "--dir",
        dir,
        "--plan-id",
        "cli-read",
        "--json",
    ]);
    let created: Value = serde_json::from_slice(&created.stdout).unwrap();
    let id = created["id"].as_str().unwrap();

    // No plan.md and no plan event.
    let missing = ws(&["loop", "plan", id]);
    assert_eq!(missing.status.code(), Some(3), "{missing:?}");
    let unknown = ws(&["loop", "plan", "no-such-loop", "--json"]);
    assert_eq!(unknown.status.code(), Some(3), "{unknown:?}");

    let text =
        "<!-- loop-plan\nplanId: cli-read\nengine: skills\n-->\n# Plan\n\nno newline at the end";
    let sha = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
    let body = json!({"text":text,"sha":sha}).to_string();
    let append = |body: &str| {
        ws(&[
            "event",
            "loop.plan",
            "--target",
            id,
            "--source",
            "tool",
            "--body",
            body,
        ])
    };
    let first = append(&body);
    assert!(first.status.success(), "{first:?}");
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();

    let printed = ws(&["loop", "plan", id]);
    assert!(printed.status.success(), "{printed:?}");
    assert_eq!(String::from_utf8(printed.stdout).unwrap(), text);
    let by_plan_id = ws(&["loop", "plan", "cli-read", "--json"]);
    let object: Value = serde_json::from_slice(&by_plan_id.stdout).unwrap();
    assert_eq!(object["seq"], first["seq"]);
    assert_eq!(object["engine"], "skills");
    let out = sandbox.home.path().join("plan-out.md");
    let written = ws(&["loop", "plan", id, "--out", out.to_str().unwrap()]);
    assert!(written.status.success(), "{written:?}");
    assert!(written.stdout.is_empty());
    assert_eq!(fs::read_to_string(&out).unwrap(), text);
    let shown = ws(&["loop", "get", id, "--json"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["engine"], "skills");
    let listed = ws(&["loop", "list", "--json"]);
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed[0]["engine"], "skills");

    // The same plan again appends nothing. A wrong sha is refused.
    let repeat = append(&body);
    assert!(repeat.status.success(), "{repeat:?}");
    let repeat: Value = serde_json::from_slice(&repeat.stdout).unwrap();
    assert_eq!(repeat["seq"], first["seq"]);
    let log = ws(&["log", id, "--type", "loop.plan"]);
    let log: Value = serde_json::from_slice(&log.stdout).unwrap();
    assert_eq!(log["entries"].as_array().unwrap().len(), 1);
    let wrong = append(&json!({"text":text,"sha":"0".repeat(64)}).to_string());
    assert!(!wrong.status.success());
    assert!(
        String::from_utf8_lossy(&wrong.stderr).contains("sha"),
        "{wrong:?}"
    );
}

#[test]
fn loop_create_appends_the_first_plan_from_plan_md_once() {
    let sandbox = Sandbox::new();
    let text = "<!-- loop-plan\nplanId: create-seed\n-->\n# Plan\n";
    fs::write(sandbox.loop_dir.join("plan.md"), text).unwrap();
    let created = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    let id = created["id"].as_str().unwrap();
    let plan = sandbox.request_get(&format!("/v1/loops/{id}/plan"));
    assert_eq!(plan["text"], text);
    assert_eq!(plan["engine"], Value::Null);
    let log = sandbox.request_get(&format!("/v1/sessions/{id}/log?type=loop.plan"));
    assert_eq!(log["entries"][0]["body"]["source"], "daemon");
    sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    let log = sandbox.request_get(&format!("/v1/sessions/{id}/log?type=loop.plan"));
    assert_eq!(log["entries"].as_array().unwrap().len(), 1);
}

#[test]
fn skills_tier_changes_the_plan_and_refuses_a_review_that_started() {
    let sandbox = Sandbox::new();
    let plan = sandbox.loop_dir.join("plan.md");
    fs::write(&plan, "# Plan\n## Phases\n### Phase 9 — Review `[kind: pr-review]` `[status: todo]` `[review: standard]` `[rounds: 2]`\n").unwrap();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
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
    assert_eq!(
        entry["body"]["sha"],
        format!("{:x}", sha2::Sha256::digest(text.as_bytes()))
    );
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
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
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
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: cli-plan\n-->\n",
    )
    .unwrap();
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
    fs::write(
        sandbox.loop_dir.join("plan.md"),
        "<!-- loop-plan\nplanId: loop-test\ndaemon: http://localhost:7717\n-->\n# Plan\nengine: skills\n",
    )
    .unwrap();
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
fn top_finds_the_loop_session_in_wsd_before_the_store_scan() {
    let sandbox = Sandbox::new();
    sandbox.dashboard();
    let created = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let other = sandbox.home.path().join("other");
    fs::create_dir_all(&other).unwrap();
    let child = sandbox.request(
        "/v1/sessions",
        json!({"headless":true,"command":["/usr/bin/true"],"cwd":other,"parent_session_id":created["id"]}),
    );
    // A store record and a state.json link name a different loop. wsd wins.
    fs::write(
        sandbox.loop_dir.join("state.json"),
        json!({"runId":"scanned-loop","wsSession":child["id"]}).to_string(),
    )
    .unwrap();
    let store = sandbox.home.path().join(".loop/loops");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("scanned-loop.json"),
        json!({"loopDir":sandbox.loop_dir}).to_string(),
    )
    .unwrap();
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .current_dir(&other)
        .args(["top", "--once"])
        .env("WS_SESSION_ID", child["id"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["args"], json!([created["id"], "--once"]));
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
fn skills_start_from_plan_only_prepares_an_agent_under_the_loop_session() {
    let sandbox = Sandbox::new();
    sandbox.skills_plan();
    fs::remove_file(sandbox.loop_dir.join("state.json")).unwrap();
    fs::remove_file(sandbox.loop_dir.join("sub/prompt-seed.md")).unwrap();
    fs::remove_file(sandbox.home.path().join("dotfile/loop-orchestrator.sh")).unwrap();
    let caller = sandbox.request("/v1/sessions", json!({}));
    let child = sandbox.request(
        "/v1/loops/start",
        json!({"loop_dir":sandbox.loop_dir,"parent_session_id":caller["id"]}),
    );
    assert_eq!(child["body"]["kind"], "headless");
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir}));
    assert_eq!(child["parent_session_id"], owner["id"]);
    assert_eq!(owner["parent_session_id"], caller["id"]);
    assert_eq!(owner["body"]["plan_id"], "loop-test");
    let other = sandbox.home.path().join("other-skills-loop");
    fs::create_dir(&other).unwrap();
    fs::copy(sandbox.loop_dir.join("plan.md"), other.join("plan.md")).unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":other}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("running orchestrator"), "{response}");
    let response = common::get(&sandbox.socket, "/v1/loops");
    let loops: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(loops.as_array().unwrap().len(), 1);
    let captured = sandbox.agent_capture();
    assert_eq!(captured["session"], child["id"]);
    assert_eq!(captured["loop"], owner["id"]);
    let argv = captured["argv"].as_array().unwrap();
    assert_eq!(argv[0], "-p");
    let prompt = argv[1].as_str().unwrap();
    assert!(prompt.contains("loop skill"), "{prompt}");
    assert!(prompt.contains(owner["id"].as_str().unwrap()), "{prompt}");
    assert!(
        prompt.contains(sandbox.loop_dir.to_str().unwrap()),
        "{prompt}"
    );
    for flag in ["--mcp-config", "--settings"] {
        let index = argv.iter().position(|arg| arg == flag).unwrap();
        assert!(PathBuf::from(argv[index + 1].as_str().unwrap()).is_file());
    }
    assert!(!sandbox.loop_dir.join("state.json").exists());
    assert!(!sandbox.loop_dir.join("sub/prompt-seed.md").exists());
    assert!(!sandbox.loop_dir.join("sub/engine-ready").exists());
}

#[test]
fn skills_start_reuses_an_advanced_loop_and_refuses_a_second_orchestrator() {
    let sandbox = Sandbox::new();
    sandbox.skills_plan();
    let state = r#"{"runId":"legacy-loop","milestone":7}"#;
    fs::write(sandbox.loop_dir.join("state.json"), state).unwrap();
    fs::remove_file(sandbox.loop_dir.join("sub/prompt-seed.md")).unwrap();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let output = common::ws(sandbox.home.path(), &sandbox.socket)
        .args([
            "loop",
            "start",
            sandbox.loop_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let child: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(child["parent_session_id"], owner["id"]);
    assert_eq!(sandbox.agent_capture()["session"], child["id"]);
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":sandbox.loop_dir}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("running orchestrator"), "{response}");
    assert_eq!(
        fs::read_to_string(sandbox.loop_dir.join("state.json")).unwrap(),
        state
    );
}

#[test]
fn skills_start_refuses_a_parked_loop_with_the_milestone_resume_command() {
    let sandbox = Sandbox::new();
    sandbox.skills_plan();
    let state = r#"{"runId":"legacy-loop","milestone":6,"awaitingMerge":{"milestone":6,"prs":[]}}"#;
    fs::write(sandbox.loop_dir.join("state.json"), state).unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/loops/start",
        &json!({"loop_dir":sandbox.loop_dir}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    let body: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("loop-milestone.sh resume --dir"),
        "{message}"
    );
    assert!(
        message.contains(sandbox.loop_dir.to_str().unwrap()),
        "{message}"
    );
    assert_eq!(
        fs::read_to_string(sandbox.loop_dir.join("state.json")).unwrap(),
        state
    );
    assert!(!sandbox.home.path().join("agent-capture.json").exists());
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
                json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())})
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
        json!({"loop_dir":"relative","plan_id":"fixture-plan"}),
        json!({"loop_dir":sandbox.home.path(),"plan_id":format!("fixture:{}", sandbox.home.path().display())}),
        json!({"loop_dir":sandbox.loop_dir,"unknown":true,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}),
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
fn loop_phases_body_check_takes_the_plan_output_kinds() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("output-loop");
    fs::create_dir_all(&directory).unwrap();
    let owner = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":directory,"plan_id":format!("fixture:{}", directory.display())}),
    );
    let id = owner["id"].as_str().unwrap();
    let post = |output: &str| {
        let phase = json!({"n":1,"title":"T","kind":"work","output":output,"milestone":1,
            "gate":null,"repo":null,"branch":null,"depends_on":[],"status":"todo"});
        common::request(
            &sandbox.socket,
            "POST",
            &format!("/v1/sessions/{id}/events"),
            &json!({"type":"loop.phases","body":{"phases":[phase]}}).to_string(),
        )
    };
    for output in ["pr", "file", "doc", "output.md"] {
        let response = post(output);
        assert!(response.starts_with("HTTP/1.1 201"), "{output}: {response}");
    }
    let response = post("document");
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
}

#[test]
fn a_started_child_marks_its_phase_running_and_an_open_merge_gate_refuses_the_next_milestone() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("gate-loop");
    fs::create_dir_all(&directory).unwrap();
    let owner = sandbox.request(
        "/v1/loops/create",
        json!({"loop_dir":directory,"plan_id":format!("fixture:{}", directory.display())}),
    );
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

#[test]
fn a_tool_opens_an_accept_gate_that_holds_dependent_spawns_until_accept() {
    let sandbox = Sandbox::new();
    let owner = sandbox.request("/v1/loops/create", json!({"loop_dir":sandbox.loop_dir,"plan_id":format!("fixture:{}", sandbox.loop_dir.display())}));
    let id = owner["id"].as_str().unwrap();
    let phase = |n: u64, kind: &str, gate: Value, depends_on: Value| {
        json!({"n":n,"title":format!("P{n}"),"kind":kind,"output":"doc","milestone":1,
            "gate":gate,"repo":null,"branch":null,"depends_on":depends_on,"status":"todo"})
    };
    let event = |kind: &str, body: Value| {
        sandbox.request(
            &format!("/v1/sessions/{id}/events"),
            json!({"type":kind,"body":body}),
        )
    };
    event(
        "loop.phases",
        json!({"phases":[
            phase(1, "work", Value::Null, json!([])),
            phase(2, "review", json!("accept"), json!([1])),
            phase(3, "work", Value::Null, json!([2])),
        ]}),
    );
    let park = || {
        let base = common::ws(sandbox.home.path(), &sandbox.socket);
        let mut command = std::process::Command::new("zsh");
        command.env_remove("WS_SESSION").env_remove("WS_SESSION_ID");
        for (key, value) in base.get_envs() {
            if let Some(value) = value {
                command.env(key, value);
            }
        }
        command
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../loop-milestone.sh"))
            .args([
                "park",
                "--kind",
                "accept",
                "--phase",
                "2",
                "--artifacts",
                "[\"notes.md\"]",
            ])
            .env("WS_BIN", env!("CARGO_BIN_EXE_ws"))
            .env("WS_LOOP_ID", id)
            .output()
            .unwrap()
    };
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
    // The doc phase and its review (final record for the repository none) reach done.
    event(
        "loop.attempt",
        json!({"n":1,"attempt":1,"session":"child","state":"result",
            "outcome":"done","verify":"none"}),
    );
    let early = park();
    assert_eq!(early.status.code(), Some(3), "{early:?}");
    event(
        "loop.review",
        json!({"phase":2,"repo":"none","key":"f","role":"final","round":1,
            "state":"finished","session":"s","provider":"claude","verdict":"APPROVE"}),
    );
    let opened = park();
    assert!(opened.status.success(), "{opened:?}");
    let repeated = park();
    assert!(repeated.status.success(), "{repeated:?}");
    let state = |sandbox: &Sandbox| {
        let response = common::get(&sandbox.socket, &format!("/v1/loops/{id}"));
        serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    assert_eq!(state(&sandbox)["phases"]["2"]["status"], "done");
    assert_eq!(state(&sandbox)["gates"].as_array().unwrap().len(), 1);
    assert_eq!(state(&sandbox)["gates"][0]["state"], "open");
    let refused = spawn(3);
    assert!(refused.starts_with("HTTP/1.1 409"), "{refused}");
    assert!(refused.contains("accept_gate_open"), "{refused}");
    let accepted = common::ws(sandbox.home.path(), &sandbox.socket)
        .env_remove("WS_SESSION")
        .env_remove("WS_SESSION_ID")
        .args(["loop", "accept", id])
        .output()
        .unwrap();
    assert!(accepted.status.success(), "{accepted:?}");
    assert!(spawn(3).starts_with("HTTP/1.1 201"));
}

impl Sandbox {
    fn new() -> Self {
        Self::with_notifications(false)
    }

    fn with_notifications(notify: bool) -> Self {
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
            .env_remove("WS_SESSION")
            .env_remove("WS_LOOP_ID")
            .env_remove("LOOP_DAEMON_URL");
        if notify {
            let adapter = home.path().join("notify");
            fs::write(
                &adapter,
                "#!/bin/sh\ncat >> \"$HOME/notifications.jsonl\"\n",
            )
            .unwrap();
            fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
            command.env("WS_NOTIFY", "1").env("WS_NOTIFIER", adapter);
        }
        let daemon = common::Daemon::start(command, &socket);
        Self {
            _daemon: daemon,
            home,
            socket,
            loop_dir,
        }
    }

    fn seed_legacy_loop(&mut self, plan: Option<&str>) -> String {
        self._daemon.child.kill().unwrap();
        self._daemon.child.wait().unwrap();
        let id = ws_core::ids::Ulid::new();
        let log = ws_core::log::SessionLog::new(
            &self.home.path().join("state"),
            id,
            ws_core::ids::Ulid::nil(),
        );
        log.append(0, "session.created", json!({"alias":100,"name":null,"workspace_id":null,
            "parent_session_id":null,"kind":"loop","loop_dir":self.loop_dir.canonicalize().unwrap()})).unwrap();
        if let Some(text) = plan {
            log.append(0, "loop.plan", json!({"text":"<!-- loop-plan\nplanId: obsolete-plan\n-->\n","sha":"old","source":"daemon"})).unwrap();
            log.append(
                0,
                "loop.plan",
                json!({"text":text,"sha":"current","source":"daemon"}),
            )
            .unwrap();
        }
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env_remove("WS_LOOP_ID")
            .env_remove("LOOP_DAEMON_URL");
        self._daemon = common::Daemon::spawn(command);
        self._daemon
            .wait_ready_for(&self.socket, std::time::Duration::from_secs(30));
        id.to_string()
    }

    fn skills_plan(&self) {
        fs::write(
            self.loop_dir.join("plan.md"),
            "<!-- loop-plan\nplanId: loop-test\ndaemon: http://localhost:7717\nengine: skills\n-->\n# Plan\n",
        )
        .unwrap();
        let agent = self.home.path().join("fake-agents/claude");
        fs::write(
            &agent,
            r#"#!/usr/bin/python3
import json, os, sys
from pathlib import Path
if sys.argv[1:] == ['--version']:
    print('2.1.302')
    sys.exit(0)
root = Path(os.environ['HOME'])
result = {'argv': sys.argv[1:], 'session': os.environ['WS_SESSION_ID'],
          'loop': os.environ.get('WS_LOOP_ID')}
temporary = root / 'agent-capture.tmp'
temporary.write_text(json.dumps(result))
temporary.rename(root / 'agent-capture.json')
with (root / 'coordinator/.loop/sub/release').open() as release:
    release.readline()
"#,
        )
        .unwrap();
    }

    fn agent_capture(&self) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let path = self.home.path().join("agent-capture.json");
        loop {
            if let Ok(bytes) = fs::read(&path) {
                return serde_json::from_slice(&bytes).unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "The agent did not start."
            );
            std::thread::yield_now();
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

    fn request_get(&self, path: &str) -> Value {
        let response = common::get(&self.socket, path);
        assert!(response.starts_with("HTTP/1.1 2"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
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
