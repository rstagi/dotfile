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
