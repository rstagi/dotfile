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
    assert!(tools.iter().any(|tool| tool.name == "session_list"));
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
    let mut mcp_keys: Vec<_> = created.as_object().unwrap().keys().collect();
    let mut cli_keys: Vec<_> = cli.as_object().unwrap().keys().collect();
    mcp_keys.sort();
    cli_keys.sort();
    assert_eq!(mcp_keys, cli_keys);
    server.finish().await;
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
        let home = tempfile::tempdir_in("/tmp").unwrap();
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
    assert_eq!(
        cli.as_object().unwrap().keys().collect::<Vec<_>>(),
        stopped.as_object().unwrap().keys().collect::<Vec<_>>()
    );
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
