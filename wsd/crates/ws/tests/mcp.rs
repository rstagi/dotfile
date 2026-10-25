#[allow(dead_code)]
mod common;

use std::time::Duration;

use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};

#[tokio::test]
async fn stdio_session_list_matches_the_json_cli_and_records_the_caller() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let response = common::request(&socket, "POST", "/v1/sessions", "{}");
    let parent: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = parent["id"].as_str().unwrap();
    let mut command = tokio::process::Command::from(common::ws(home.path(), &socket));
    command
        .arg("mcp")
        .env("WS_SESSION_ID", id)
        .env_remove("WS_SESSION");
    let client = tokio::time::timeout(
        Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "loop_accept",
            "loop_create",
            "loop_get",
            "loop_list",
            "loop_model",
            "loop_note",
            "loop_pause",
            "loop_reject",
            "loop_resume",
            "loop_start",
            "loop_stop",
            "loop_tier",
            "phase_spawn",
            "session_block_on",
            "session_create",
            "session_depends_on",
            "session_event",
            "session_get",
            "session_list",
            "session_log",
            "session_output",
            "session_rename",
            "session_send",
            "session_stop",
            "session_tail",
            "wait"
        ]
    );
    for tool in &tools {
        assert_eq!(tool.input_schema["additionalProperties"], false);
        assert!(
            tool.input_schema
                .get("properties")
                .and_then(|properties| properties.get("parent_session_id"))
                .is_none()
        );
    }
    let result = client
        .call_tool(
            CallToolRequestParams::new("session_list")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let output = common::ws(home.path(), &socket)
        .args(["session", "list", "--json"])
        .env("WS_SESSION_ID", id)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = result.content[0].as_text().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text.text).unwrap(), cli);
    let entries = ws_core::log::SessionLog::new(
        &home.path().join("state"),
        id.parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    )
    .read()
    .unwrap()
    .entries;
    assert!(
        entries
            .iter()
            .any(|entry| entry.body["operation"] == "session.list"
                && entry.body["parent_session_id"] == id)
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn session_create_and_get_match_the_json_cli() {
    let server = TestServer::start().await;
    let created = server
        .call("session_create", json!({"name":"mcp-child"}))
        .await;
    assert_eq!(created["parent_session_id"], server.parent);
    let id = created["id"].as_str().unwrap();
    assert_eq!(created, server.cli(&["session", "get", id, "--json"]));
    let fetched = server
        .call(
            "session_get",
            json!({"target":format!("#{}", created["alias"])}),
        )
        .await;
    assert_eq!(fetched, created);
    let cli = server.cli(&["session", "create", "--name", "cli-child", "--json"]);
    assert_eq!(cli["parent_session_id"], server.parent);
    assert_eq!(
        cli,
        server
            .call("session_get", json!({"target":cli["id"]}))
            .await
    );
    let mut expected = canonical_session(created);
    expected["name"] = json!("cli-child");
    expected["body"]["name"] = json!("cli-child");
    assert_eq!(canonical_session(cli), expected);
    server.assert_audit("session.create");
    server.assert_audit("session.get");
    server.finish().await;
}

#[tokio::test]
async fn session_rename_renames_only_the_caller_and_reports_clear_errors() {
    let server = TestServer::start().await;
    let other = server
        .call("session_create", json!({"name":"taken-name"}))
        .await;
    let renamed = server
        .call("session_rename", json!({"name":"fix-login-bug"}))
        .await;
    assert_eq!(renamed["id"], server.parent);
    assert_eq!(renamed["name"], "fix-login-bug");
    assert_eq!(
        server.cli(&["session", "get", &server.parent, "--json"])["name"],
        "fix-login-bug"
    );
    for (name, code) in [
        ("1-bad name", "invalid_request"),
        ("taken-name", "name_taken"),
    ] {
        let error = server.error("session_rename", json!({"name":name})).await;
        assert_eq!(error["error"]["code"], code, "{error}");
        assert!(
            error["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("name")),
            "{error}"
        );
    }
    let rejected = server
        .error("session_rename", json!({"name":"x", "target":other["id"]}))
        .await;
    assert!(
        rejected["error"]["message"].as_str().is_some(),
        "{rejected}"
    );
    assert_eq!(
        server.cli(&["session", "get", other["id"].as_str().unwrap(), "--json"])["name"],
        "taken-name"
    );
    server.finish().await;
}

#[tokio::test]
async fn session_rename_requires_a_caller_identity() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("ws.sock");
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let mut command = tokio::process::Command::from(common::ws(home.path(), &socket));
    command.arg("mcp");
    let client = tokio::time::timeout(
        Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("session_rename")
                .with_arguments(json!({"name":"orphan"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let error = result.structured_content.unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("WS_SESSION_ID")),
        "{error}"
    );
    client.cancel().await.unwrap();
}

struct TestServer {
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    _daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: std::path::PathBuf,
    parent: String,
}

impl TestServer {
    async fn start() -> Self {
        Self::with_config("").await
    }

    async fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::create_dir(home.path().join("state")).unwrap();
        std::fs::write(home.path().join("state/config.toml"), config).unwrap();
        let socket = home.path().join("ws.sock");
        let daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
        let response = common::request(&socket, "POST", "/v1/sessions", "{}");
        let parent: Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        let parent = parent["id"].as_str().unwrap().to_owned();
        let mut command = tokio::process::Command::from(common::ws(home.path(), &socket));
        command
            .arg("mcp")
            .env("WS_SESSION_ID", &parent)
            .env_remove("WS_SESSION");
        let client = tokio::time::timeout(
            Duration::from_secs(30),
            ().serve(TokioChildProcess::new(command).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        Self {
            client,
            _daemon: daemon,
            home,
            socket,
            parent,
        }
    }

    async fn call(&self, name: &str, arguments: Value) -> Value {
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.client.call_tool(
                CallToolRequestParams::new(name.to_owned())
                    .with_arguments(arguments.as_object().unwrap().clone()),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let value: Value =
            serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
        assert_eq!(result.structured_content.as_ref(), Some(&value));
        value
    }

    async fn error(&self, name: &str, arguments: Value) -> Value {
        let result = self
            .client
            .call_tool(
                CallToolRequestParams::new(name.to_owned())
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
        result.structured_content.unwrap()
    }

    fn cli(&self, arguments: &[&str]) -> Value {
        let output = common::ws(self.home.path(), &self.socket)
            .args(arguments)
            .env("WS_SESSION_ID", &self.parent)
            .env_remove("WS_SESSION")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    async fn finish(self) {
        self.client.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn session_stop_matches_the_json_cli_and_records_the_caller() {
    let server = TestServer::start().await;
    let first = server.cli(&["session", "create", "--json"]);
    let second = server.cli(&["session", "create", "--json"]);
    let stopped = server
        .call("session_stop", json!({"target":first["id"]}))
        .await;
    assert_eq!(stopped["state"], "finished");
    assert_eq!(
        stopped,
        server.cli(&["session", "get", first["id"].as_str().unwrap(), "--json"])
    );
    let cli = server.cli(&["session", "stop", second["id"].as_str().unwrap(), "--json"]);
    assert_eq!(cli["state"], stopped["state"]);
    assert_eq!(canonical_session(cli), canonical_session(stopped));
    server.assert_audit("session.stop");
    server.finish().await;
}

impl TestServer {
    fn assert_audit(&self, operation: &str) {
        let entries = ws_core::log::SessionLog::new(
            &self.home.path().join("state"),
            self.parent.parse().unwrap(),
            ws_core::ids::Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries;
        assert!(
            entries
                .iter()
                .any(|entry| entry.body["operation"] == operation
                    && entry.body["parent_session_id"] == self.parent),
            "No audit record for {operation}."
        );
    }
}

#[tokio::test]
async fn session_send_matches_the_json_cli_and_records_the_caller() {
    let server = TestServer::start().await;
    let recipient = server.cli(&["session", "create", "--json"]);
    let id = recipient["id"].as_str().unwrap();
    let sent = server
        .call(
            "session_send",
            json!({"target":id,"body":"a message with quotes: ' \" and\na newline"}),
        )
        .await;
    assert_eq!(sent["from_session"], server.parent);
    assert_eq!(sent["to_session"], id);
    let output = common::ws(server.home.path(), &server.socket)
        .args(["send", id, sent["body"].as_str().unwrap(), "--json"])
        .env("WS_SESSION_ID", &server.parent)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let mut cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sent["pair_seq"], 1);
    assert_eq!(cli["pair_seq"], 2);
    assert_ne!(sent["event_id"], cli["event_id"]);
    let mut expected = sent.clone();
    for value in [&mut cli, &mut expected] {
        value.as_object_mut().unwrap().remove("event_id");
        value.as_object_mut().unwrap().remove("pair_seq");
    }
    assert_eq!(cli, expected);
    server.assert_audit("message.send");
    server.finish().await;
}

#[tokio::test]
async fn dependency_tools_match_the_json_cli_and_record_the_caller() {
    let server = TestServer::start().await;
    for (tool, command, kind) in [
        ("session_depends_on", "depends-on", "depends_on"),
        ("session_block_on", "block-on", "block_on"),
    ] {
        let source = server.cli(&["session", "create", "--json"]);
        let target = server.cli(&["session", "create", "--json"]);
        let edge = server
            .call(
                tool,
                json!({"target":source["id"], "dependency":target["id"]}),
            )
            .await;
        let cli = server.cli(&[
            "session",
            command,
            source["id"].as_str().unwrap(),
            target["id"].as_str().unwrap(),
            "--json",
        ]);
        assert_eq!(edge["source"], source["id"]);
        assert_eq!(edge["target"], target["id"]);
        assert_eq!(edge["kind"], kind);
        assert_eq!(edge["body"]["parent_session_id"], server.parent);
        let mut expected = edge;
        let mut cli = cli;
        expected.as_object_mut().unwrap().remove("id");
        cli.as_object_mut().unwrap().remove("id");
        assert_eq!(cli, expected);
    }
    server.assert_audit("session.edge");
    server.finish().await;
}

#[tokio::test]
async fn wait_matches_the_json_cli_and_records_the_caller() {
    let server = TestServer::start().await;
    let child = server.cli(&["session", "create", "--json"]);
    let id = child["id"].as_str().unwrap();
    let result = server
        .call(
            "wait",
            json!({"target":id, "condition":"state=created", "timeout":0}),
        )
        .await;
    assert_eq!(result["session"]["id"], id);
    let cli = server.cli(&[
        "wait",
        id,
        "--for",
        "state=created",
        "--timeout",
        "0",
        "--json",
    ]);
    assert_eq!(cli, result);
    server.assert_audit("session.wait");
    server.finish().await;
}

#[tokio::test]
async fn session_create_passes_the_key_so_a_retry_returns_the_first_session() {
    let server = TestServer::start().await;
    let request = json!({"headless":true,"key":"mcp-key","command":["/bin/sh","-c","true"],"cwd":server.home.path()});
    let first = server.call("session_create", request.clone()).await;
    let retry = server.call("session_create", request).await;
    assert_eq!(retry["id"], first["id"]);
    assert_eq!(first["body"]["key"], "mcp-key");
    server
        .call(
            "wait",
            json!({"target":first["id"],"condition":"exit","timeout":30}),
        )
        .await;
    server.finish().await;
}

#[tokio::test]
async fn session_create_launches_a_headless_process_with_cli_json_parity() {
    let server = TestServer::start().await;
    let child = server.call("session_create", json!({"headless":true, "command":["/bin/sh","-c","printf mcp-output"], "cwd":server.home.path()})).await;
    assert_eq!(child["parent_session_id"], server.parent);
    let id = child["id"].as_str().unwrap();
    let finished = server
        .call("wait", json!({"target":id,"condition":"exit","timeout":30}))
        .await;
    assert_eq!(finished["session"]["exit_code"], 0);
    let cli = server.cli(&[
        "session",
        "create",
        "--headless",
        "--json",
        "--cwd",
        server.home.path().to_str().unwrap(),
        "--",
        "/bin/sh",
        "-c",
        "printf mcp-output",
    ]);
    assert_eq!(cli["parent_session_id"], server.parent);
    assert_eq!(cli["body"]["kind"], "headless");
    assert_eq!(
        canonical_session(cli.clone()),
        canonical_session(child.clone())
    );
    server
        .call(
            "wait",
            json!({"target":cli["id"],"condition":"exit","timeout":30}),
        )
        .await;
    server.finish().await;
}

#[tokio::test]
async fn session_tail_is_bounded_and_matches_the_json_cli() {
    let server = TestServer::start().await;
    let child = server.call("session_create", json!({"headless":true,"command":["/bin/sh","-c","printf 0123456789abcdef; printf error-text >&2"],"cwd":server.home.path()})).await;
    let id = child["id"].as_str().unwrap();
    server
        .call("wait", json!({"target":id,"condition":"exit","timeout":30}))
        .await;
    let tail = server
        .call("session_tail", json!({"target":id,"max_bytes":8}))
        .await;
    assert_eq!(tail["output"], "89abcdef");
    assert_eq!(tail["truncated"], true);
    assert_eq!(
        tail,
        server.cli(&["session", "tail", id, "--json", "--max-bytes", "8"])
    );
    let stderr = server
        .call("session_tail", json!({"target":id,"stream":"stderr"}))
        .await;
    assert_eq!(stderr["output"], "error-text");
    assert_eq!(stderr["truncated"], false);
    assert_eq!(
        stderr,
        server.cli(&["session", "tail", id, "--json", "--stream", "stderr"])
    );
    server.assert_audit("session.tail");
    server.finish().await;
}

#[tokio::test]
async fn phase_spawn_matches_the_json_cli_and_records_the_caller() {
    let server = TestServer::start().await;
    let child = server.call("phase_spawn", json!({"phase":{"command":["/bin/sh","-c","true"]},"repo":server.home.path(),"base":"main","caps_required":[],"max_runtime":30})).await;
    assert_eq!(child["parent_session_id"], server.parent);
    assert_eq!(child["body"]["base"], "main");
    let cli = server.cli(&[
        "phase",
        "spawn",
        "--repo",
        server.home.path().to_str().unwrap(),
        "--base",
        "main",
        "--max-runtime",
        "30",
        "--json",
        "--",
        "/bin/sh",
        "-c",
        "true",
    ]);
    assert_eq!(cli["parent_session_id"], server.parent);
    assert_eq!(cli["body"]["phase"], child["body"]["phase"]);
    assert_eq!(cli["body"]["caps_required"], child["body"]["caps_required"]);
    assert_eq!(
        canonical_session(cli.clone()),
        canonical_session(child.clone())
    );
    for id in [&child["id"], &cli["id"]] {
        let finished = server
            .call("wait", json!({"target":id,"condition":"exit","timeout":30}))
            .await;
        assert_eq!(finished["session"]["exit_code"], 0);
    }
    server.assert_audit("phase.spawn");
    server.finish().await;
}

#[tokio::test]
async fn daemon_errors_remain_structured_and_do_not_stop_the_mcp_server() {
    let server = TestServer::start().await;
    let response = server
        .client
        .call_tool(
            CallToolRequestParams::new("session_get")
                .with_arguments(json!({"target":"missing"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(response.is_error, Some(true));
    let error = response.structured_content.unwrap();
    assert_eq!(error["error"]["code"], "session_not_found");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    let invalid = server
        .client
        .call_tool(
            CallToolRequestParams::new("session_create").with_arguments(
                json!({"parent_session_id":server.parent})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(invalid.is_error, Some(true));
    let listed = server.call("session_list", json!({})).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    server.finish().await;
}

#[tokio::test]
async fn a_pending_wait_allows_other_mcp_tools_to_run() {
    let server = TestServer::start().await;
    let child = server.cli(&["session", "create", "--json"]);
    let id = child["id"].as_str().unwrap();
    let wait = server.call("wait", json!({"target":id,"condition":"state=finished"}));
    let complete = async {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let entries = ws_core::log::SessionLog::new(
                &server.home.path().join("state"),
                server.parent.parse().unwrap(),
                ws_core::ids::Ulid::nil(),
            )
            .read()
            .unwrap()
            .entries;
            if entries
                .iter()
                .any(|entry| entry.body["operation"] == "session.wait")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "The daemon did not register the wait."
            );
            tokio::task::yield_now().await;
        }
        let sessions = server.call("session_list", json!({})).await;
        assert_eq!(sessions.as_array().unwrap().len(), 2);
        server.call("session_stop", json!({"target":id})).await
    };
    let (waited, stopped) = tokio::join!(wait, complete);
    assert_eq!(waited["session"]["id"], stopped["id"]);
    assert_eq!(waited["session"]["state"], "finished");
    server.finish().await;
}

fn canonical_session(mut session: Value) -> Value {
    let id = session["id"].as_str().unwrap().to_owned();
    session["id"] = json!("<session>");
    session["alias"] = json!(0);
    session["created_at_ms"] = json!(0);
    session["updated_at_ms"] = json!(0);
    session["body"]["alias"] = json!(0);
    for stream in ["stdout", "stderr"] {
        if let Some(path) = session["body"][stream].as_str() {
            session["body"][stream] = json!(path.replace(&id, "<session>"));
        }
    }
    session
}

#[tokio::test]
async fn session_tail_enforces_default_and_hard_limits_after_utf8_decoding() {
    let server = TestServer::start().await;
    let child = server.call("session_create", json!({"headless":true,"command":["/bin/sh","-c","i=0; while [ \"$i\" -lt 7000 ]; do printf abcdefghij; i=$((i+1)); done; printf '\\377\\376\\377'"],"cwd":server.home.path()})).await;
    let id = child["id"].as_str().unwrap();
    server
        .call("wait", json!({"target":id,"condition":"exit","timeout":30}))
        .await;
    for (arguments, limit) in [
        (json!({"target":id}), 8192),
        (json!({"target":id,"max_bytes":65536}), 65536),
        (json!({"target":id,"max_bytes":2}), 2),
    ] {
        let result = server.call("session_tail", arguments).await;
        assert!(result["output"].as_str().unwrap().len() <= limit);
        assert_eq!(result["truncated"], true);
    }
    for limit in [0, 65537] {
        let result = server
            .client
            .call_tool(
                CallToolRequestParams::new("session_tail").with_arguments(
                    json!({"target":id,"max_bytes":limit})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.structured_content.unwrap()["error"]["code"],
            "invalid_request"
        );
    }
    server.finish().await;
}

#[tokio::test]
async fn json_send_uses_ws_session_id_without_resolving_a_legacy_session() {
    let server = TestServer::start().await;
    let recipient = server.cli(&["session", "create", "--json"]);
    let output = common::ws(server.home.path(), &server.socket)
        .args([
            "send",
            recipient["id"].as_str().unwrap(),
            "from a terminal",
            "--json",
        ])
        .env_remove("WS_SESSION_ID")
        .env("WS_SESSION", "unrelated-legacy-session")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let sent: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sent["to_session"], recipient["id"]);
    assert_eq!(sent["body"], "from a terminal");
    assert_ne!(sent["from_session"], server.parent);
    server.finish().await;
}

#[tokio::test]
async fn record_creation_rejects_process_options_without_headless() {
    let server = TestServer::start().await;
    for option in ["--parent", "--stdin", "--stdout", "--stderr", "--env"] {
        let output = common::ws(server.home.path(), &server.socket)
            .args(["session", "create", "--json", option, "unused"])
            .env("WS_SESSION_ID", &server.parent)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "The command ignored {option}: {output:?}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("--headless"));
    }
    assert_eq!(
        server
            .call("session_list", json!({}))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.finish().await;
}

#[tokio::test]
async fn spawn_limit_errors_match_in_the_cli_and_mcp_without_starting_children() {
    for (setting, code) in [
        ("max_children", "max_children_exceeded"),
        ("max_depth", "max_depth_exceeded"),
    ] {
        let server = TestServer::with_config(&format!("[limits]\n{setting} = 0\n")).await;
        let output = common::ws(server.home.path(), &server.socket)
            .env("WS_SESSION_ID", &server.parent)
            .args(["session", "create", "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let cli_error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(cli_error["error"]["code"], code);
        assert_eq!(cli_error["error"]["parent_session_id"], server.parent);
        assert_eq!(cli_error["error"]["limit"], 0);
        for (tool, arguments) in [
            ("session_create", json!({})),
            (
                "session_create",
                json!({"headless":true,"command":["/usr/bin/true"],"cwd":server.home.path()}),
            ),
            (
                "phase_spawn",
                json!({"repo":server.home.path(),"base":"main","max_runtime":30,"phase":{"command":["/usr/bin/true"]}}),
            ),
        ] {
            let response = server
                .client
                .call_tool(
                    CallToolRequestParams::new(tool)
                        .with_arguments(arguments.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            assert_eq!(response.is_error, Some(true));
            assert_eq!(response.structured_content.unwrap(), cli_error);
        }
        let output = common::ws(server.home.path(), &server.socket)
            .env("WS_SESSION_ID", &server.parent)
            .args([
                "phase",
                "spawn",
                "--repo",
                server.home.path().to_str().unwrap(),
                "--base",
                "main",
                "--max-runtime",
                "30",
                "--json",
                "--",
                "/usr/bin/true",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap(),
            cli_error
        );
        assert_eq!(
            server
                .call("session_list", json!({}))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn loop_stop_matches_the_json_cli_and_rejects_a_forged_caller() {
    let server = TestServer::start().await;
    let directory = server.home.path().join("coordinator/.loop");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("state.json"), r#"{"runId":"mcp-loop"}"#).unwrap();
    let result = server
        .call("loop_stop", json!({"loop_dir":directory}))
        .await;
    assert_eq!(result["paused"], true);
    assert_eq!(result["run_id"], "mcp-loop");
    assert_eq!(
        result,
        server.cli(&["loop", "stop", directory.to_str().unwrap(), "--json"])
    );
    let forged = server
        .client
        .call_tool(
            CallToolRequestParams::new("loop_stop").with_arguments(
                json!({"loop_dir":directory,"parent_session_id":server.parent})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(forged.is_error, Some(true));
    server.assert_audit("loop.stop");
    server.finish().await;
}

#[tokio::test]
async fn event_and_log_tools_match_the_cli_and_record_the_caller() {
    let server = TestServer::start().await;
    let appended = server
        .call(
            "session_event",
            json!({"target":"me","type":"loop.note","body":{"text":"from mcp"}}),
        )
        .await;
    assert_eq!(appended["type"], "loop.note");
    assert_eq!(appended["body"]["caller"], server.parent);
    let cli = server.cli(&[
        "event",
        "loop.decision",
        "--body",
        r#"{"topic":"t","default":1,"choice":2,"reason":"from cli"}"#,
    ]);
    assert_eq!(cli["body"]["caller"], server.parent);
    let rejected = server
        .error(
            "session_event",
            json!({"target":"me","type":"session.done"}),
        )
        .await;
    assert_eq!(rejected["error"]["code"], "invalid_request");
    let logged = server
        .call("session_log", json!({"target":"me","type":"loop."}))
        .await;
    let types: Vec<_> = logged["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["loop.note", "loop.decision"]);
    assert_eq!(logged, server.cli(&["log", "--type", "loop."]));
    let since = appended["seq"].as_u64().unwrap().to_string();
    let later = server.cli(&["log", "--type", "loop.", "--since", &since]);
    assert_eq!(later["entries"].as_array().unwrap().len(), 1);
    server.assert_audit("session.event");
    server.assert_audit("session.log");
    server.finish().await;
}

#[tokio::test]
async fn session_output_returns_published_text_with_a_utf8_safe_cap() {
    let server = TestServer::start().await;
    let child = server.call("session_create", json!({})).await;
    let id = child["id"].as_str().unwrap();
    for text in [
        "Report\nwith a final verdict.\n".to_owned(),
        format!("{}éz", "a".repeat(65535)),
    ] {
        let response = common::request(
            &server.socket,
            "POST",
            &format!("/v1/sessions/{id}/done"),
            &json!({"output":text}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let result = server.call("session_output", json!({"target":id})).await;
        let expected = if text.len() > 65536 {
            "a".repeat(65535)
        } else {
            text.clone()
        };
        assert_eq!(result["output"], expected);
        assert_eq!(result["session_id"], id);
        assert_eq!(result["truncated"], text.len() > 65536);
        assert_eq!(result, server.cli(&["session", "output", id, "--json"]));
        let plain = common::ws(server.home.path(), &server.socket)
            .args(["session", "output", id])
            .output()
            .unwrap();
        assert!(plain.status.success(), "{plain:?}");
        assert_eq!(String::from_utf8(plain.stdout).unwrap(), expected);
    }
    server.assert_audit("session.output");
    server.finish().await;
}

#[tokio::test]
async fn multiwait_matches_the_cli_and_filters_each_target_by_sequence() {
    let server = TestServer::start().await;
    let first = server.call("session_create", json!({})).await;
    let second = server.call("session_create", json!({})).await;
    let a = first["id"].as_str().unwrap();
    let b = second["id"].as_str().unwrap();
    let old = server
        .call(
            "session_event",
            json!({"target":a,"type":"loop.note","body":{"text":"n"}}),
        )
        .await;
    server
        .call(
            "session_event",
            json!({"target":b,"type":"loop.note","body":{"text":"n"}}),
        )
        .await;
    let targets = json!([{"target":a,"after_seq":old["seq"]},{"target":b,"after_seq":0}]);
    let result = server
        .call(
            "wait",
            json!({"targets":targets,"mode":"any","condition":"event=loop.note","timeout":0}),
        )
        .await;
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["matches"][0]["session"]["id"], b);
    let seq = old["seq"].as_u64().unwrap().to_string();
    assert_eq!(
        result,
        server.cli(&[
            "wait",
            a,
            b,
            "--mode",
            "any",
            "--for",
            "event=loop.note",
            "--after-seq",
            &seq,
            "--after-seq",
            "0",
            "--timeout",
            "0",
            "--json"
        ])
    );
    let error = server
        .error(
            "wait",
            json!({"targets":targets,"mode":"all","condition":"event=loop.note","timeout":0}),
        )
        .await;
    assert_eq!(error["error"]["code"], "wait_timeout");
    server
        .call(
            "session_event",
            json!({"target":a,"type":"loop.note","body":{"text":"n"}}),
        )
        .await;
    let all = server
        .call(
            "wait",
            json!({"targets":targets,"mode":"all","condition":"event=loop.note","timeout":0}),
        )
        .await;
    assert_eq!(all["matches"].as_array().unwrap().len(), 2);
    server.finish().await;
}

#[tokio::test]
async fn loop_reads_match_http_and_json_cli_and_reject_a_forged_caller() {
    let server = TestServer::start().await;
    let directory = server.home.path().join("loop");
    std::fs::create_dir(&directory).unwrap();
    let created = server
        .call("loop_create", json!({"loop_dir":directory}))
        .await;
    let id = created["id"].as_str().unwrap();
    server
        .call(
            "session_event",
            json!({"target":id,"type":"loop.phase",
        "body":{"n":109,"status":"done","attempt":1,"session":null}}),
        )
        .await;
    let fetched = server.call("loop_get", json!({"id":id})).await;
    assert_eq!(fetched["phases"]["109"]["status"], "done");
    assert_eq!(fetched, server.cli(&["loop", "get", id, "--json"]));
    let listed = server.call("loop_list", json!({})).await;
    assert_eq!(listed, json!([fetched]));
    assert_eq!(listed, server.cli(&["loop", "list", "--json"]));
    for (path, expected) in [
        (format!("/v1/loops/{id}"), &fetched),
        ("/v1/loops".into(), &listed),
    ] {
        let response = common::request(&server.socket, "GET", &path, "");
        let http: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(&http, expected);
    }
    for (tool, args) in [
        ("loop_get", json!({"id":id,"parent_session_id":id})),
        ("loop_list", json!({"parent_session_id":id})),
    ] {
        assert!(server.error(tool, args).await["error"].is_object());
    }
    assert_eq!(
        server.error("loop_get", json!({"id":server.parent})).await["error"]["code"],
        "invalid_request"
    );
    server.assert_audit("loop.get");
    server.assert_audit("loop.list");
    server.finish().await;
}

#[tokio::test]
async fn skills_loop_controls_use_the_same_routes_and_reject_a_forged_caller() {
    let server = TestServer::start().await;
    let directory = server.home.path().join("loop");
    std::fs::create_dir(&directory).unwrap();
    let created = server
        .call("loop_create", json!({"loop_dir":directory}))
        .await;
    let id = created["id"].as_str().unwrap();
    for (tool, args, operation) in [
        ("loop_pause", json!({"id":id,"phase":3}), "loop.pause"),
        ("loop_resume", json!({"id":id,"phase":3}), "loop.resume"),
        (
            "loop_model",
            json!({"id":id,"phase":3,"model":"codex:@sol+high"}),
            "loop.model",
        ),
        (
            "loop_note",
            json!({"id":id,"phase":3,"text":"Use the public API."}),
            "loop.note",
        ),
    ] {
        let entry = server.call(tool, args.clone()).await;
        assert_eq!(entry["body"]["phase"], 3);
        let mut forged = args;
        forged["parent_session_id"] = json!(id);
        assert!(server.error(tool, forged).await["error"].is_object());
        server.assert_audit(operation);
    }
    for (tool, state) in [("loop_accept", "accepted"), ("loop_reject", "rejected")] {
        server
            .call(
                "session_event",
                json!({"target":id,"type":"loop.gate",
            "body":{"kind":"accept","state":"open","milestone":2,"phase":4,"artifacts":[]}}),
            )
            .await;
        let gate = server.call(tool, json!({"id":id})).await;
        assert_eq!(gate["body"]["state"], state);
        assert_eq!(gate["body"]["source"], "daemon");
        assert!(server.error(tool, json!({"id":id})).await["error"].is_object());
        assert!(
            server
                .error(tool, json!({"id":id,"parent_session_id":id}))
                .await["error"]
                .is_object()
        );
        server
            .call(
                "session_event",
                json!({"target":id,"type":"loop.gate",
            "body":{"kind":"accept","state":"open","milestone":2,"phase":4,"artifacts":[]}}),
            )
            .await;
        let cli = server.cli(&["loop", tool.strip_prefix("loop_").unwrap(), id, "--json"]);
        assert_eq!(cli["body"]["state"], state);
    }
    let scripts = server.home.path().join("dotfile");
    std::fs::create_dir_all(&scripts).unwrap();
    std::os::unix::fs::symlink(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../loop-plan-tier.mjs")
            .canonicalize()
            .unwrap(),
        scripts.join("loop-plan-tier.mjs"),
    )
    .unwrap();
    std::fs::write(
        directory.join("plan.md"),
        "# Plan\n## Phases\n### Phase 4 — Review `[kind: pr-review]` `[status: todo]`\n",
    )
    .unwrap();
    let plan = server
        .call(
            "loop_tier",
            json!({"id":id,"phase":4,"tier":"standard","rounds":2}),
        )
        .await;
    assert_eq!(plan["type"], "loop.plan");
    assert!(
        plan["body"]["text"]
            .as_str()
            .unwrap()
            .contains("[rounds: 2]")
    );
    assert!(
        server
            .error(
                "loop_tier",
                json!({"id":id,"phase":4,"tier":"max","parent_session_id":id})
            )
            .await["error"]
            .is_object()
    );
    server.assert_audit("loop.tier");
    server.finish().await;
}
