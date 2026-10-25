//! A keyed `loop.*` append records its fact one time: through the route, the CLI and MCP,
//! and across a daemon restart.
mod common;

use serde_json::{Value, json};

use common::{Daemon, request, ws};

#[test]
fn a_repeated_key_through_the_route_appends_one_event() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let id = create_session(&socket);

    let (status, first) = append(&socket, &id, json!({"text":"a"}), Some("note:1"));
    assert_eq!(status, 201, "{first}");
    let (status, again) = append(&socket, &id, json!({"text":"changed"}), Some("note:1"));
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["seq"], first["seq"]);
    assert_eq!(again["body"]["text"], "a", "the first event stays");
    let (status, other) = append(&socket, &id, json!({"text":"b"}), Some("note:2"));
    assert_eq!(status, 201, "{other}");
    let (status, unkeyed) = append(&socket, &id, json!({"text":"c"}), None);
    assert_eq!(status, 201, "{unkeyed}");
    assert_eq!(notes(&socket, &id).len(), 3);
}

#[test]
fn a_body_cannot_set_the_key_and_a_bad_key_is_refused() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let id = create_session(&socket);

    let (status, _) = append(&socket, &id, json!({"text":"a","event_key":"k"}), None);
    assert_eq!(status, 400);
    let (status, _) = append(&socket, &id, json!({"text":"a"}), Some(""));
    assert_eq!(status, 400);
    let (status, _) = append(&socket, &id, json!({"text":"a"}), Some(&"k".repeat(257)));
    assert_eq!(status, 400);
    assert!(notes(&socket, &id).is_empty());
}

#[test]
fn a_repeated_key_through_the_cli_appends_one_event() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let id = create_session(&socket);

    let run = |text: &str| -> Value {
        let output = ws(sandbox.path(), &socket)
            .args(["event", "loop.note", "--body"])
            .arg(json!({"text":text}).to_string())
            .args(["--source", "tool", "--key", "cli:1", "--target", &id])
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let first = run("a");
    let again = run("b");
    assert_eq!(again["seq"], first["seq"]);
    assert_eq!(notes(&socket, &id).len(), 1);
}

#[tokio::test]
async fn a_repeated_key_through_mcp_appends_one_event() {
    use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let id = create_session(&socket);

    let mut command = tokio::process::Command::from(ws(sandbox.path(), &socket));
    command.arg("mcp");
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        ().serve(TokioChildProcess::new(command).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let mut seqs = Vec::new();
    for text in ["a", "b"] {
        let arguments = json!({"target":id,"type":"loop.note","body":{"text":text},
            "source":"tool","key":"mcp:1"});
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.call_tool(
                CallToolRequestParams::new("session_event")
                    .with_arguments(arguments.as_object().unwrap().clone()),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        seqs.push(result.structured_content.unwrap()["seq"].clone());
    }
    assert_eq!(seqs[0], seqs[1]);
    assert_eq!(notes(&socket, &id).len(), 1);
    client.cancel().await.unwrap();
}

#[test]
fn a_repeated_key_after_a_daemon_restart_appends_nothing() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let socket = sandbox.path().join("wsd.sock");
    let daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let id = create_session(&socket);
    let (status, first) = append(&socket, &id, json!({"text":"a"}), Some("restart:1"));
    assert_eq!(status, 201, "{first}");

    drop(daemon);
    // Daemon::start waits for the socket ping before the next request.
    let _restarted = Daemon::start(ws(sandbox.path(), &socket), &socket);
    let (status, again) = append(&socket, &id, json!({"text":"b"}), Some("restart:1"));
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["seq"], first["seq"]);
    let (status, _) = append(&socket, &id, json!({"text":"c"}), Some("restart:2"));
    assert_eq!(status, 201);
    assert_eq!(notes(&socket, &id).len(), 2);
}

fn create_session(socket: &std::path::Path) -> String {
    let response = request(socket, "POST", "/v1/sessions", "{}");
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let created: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    created["id"].as_str().unwrap().to_string()
}

fn append(socket: &std::path::Path, id: &str, body: Value, key: Option<&str>) -> (u16, Value) {
    let response = request(
        socket,
        "POST",
        &format!("/v1/sessions/{id}/events"),
        &json!({"type":"loop.note","body":body,"source":"tool","key":key}).to_string(),
    );
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1;
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

fn notes(socket: &std::path::Path, id: &str) -> Vec<Value> {
    let response = common::get(socket, &format!("/v1/sessions/{id}/log?type=loop.note"));
    let log: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    log["entries"].as_array().unwrap().clone()
}
