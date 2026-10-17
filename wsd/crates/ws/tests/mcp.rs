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
