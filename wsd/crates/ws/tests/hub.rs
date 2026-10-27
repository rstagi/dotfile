use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

mod common;
use common::{
    Daemon,
    hub_fixture::{config_hub, config_hub_with, pki, serve},
    ws,
};

fn cli(home: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = ws(home, &home.join("wsd.sock"));
    command.args(arguments).output().unwrap()
}

#[test]
fn hub_commands_without_hub_section_print_one_error_and_exit_2() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    for arguments in [
        vec!["device", "list"],
        vec!["device", "remove", "x"],
        vec!["sessions", "--all-devices"],
        vec!["sessions", "--all-devices", "--json"],
    ] {
        let output = cli(sandbox.path(), &arguments);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr).lines().count(), 1);
    }
}

#[test]
fn wsd_starts_with_and_without_hub_section() {
    for with_hub in [false, true] {
        let sandbox = tempfile::tempdir_in("/tmp").unwrap();
        let socket = sandbox.path().join("wsd.sock");
        if with_hub {
            config_hub(sandbox.path(), "127.0.0.1:1", "dev-a");
        }
        let _daemon = Daemon::start(ws(sandbox.path(), &socket), &socket);
    }
}

#[test]
fn device_commands_list_and_revoke_through_the_hub() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    for id in ["dev-a", "dev-b"] {
        let output = cli(
            dir,
            &[
                "hub",
                "add-device",
                "--state",
                dir.join("hub").to_str().unwrap(),
                "--cert",
                dir.join(format!("{id}.pem")).to_str().unwrap(),
                "--name",
                id,
            ],
        );
        assert!(output.status.success(), "{output:?}");
    }
    config_hub(dir, &hub.addr, "dev-a");
    let listed = cli(dir, &["device", "list", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    let removed = cli(dir, &["device", "remove", "dev-b"]);
    assert!(removed.status.success(), "{removed:?}");
    config_hub(dir, &hub.addr, "dev-b");
    let denied = cli(dir, &["device", "list"]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("403"),
        "{denied:?}"
    );
}

fn add_device(dir: &Path, id: &str) {
    let output = cli(
        dir,
        &[
            "hub",
            "add-device",
            "--state",
            dir.join("hub").to_str().unwrap(),
            "--cert",
            dir.join(format!("{id}.pem")).to_str().unwrap(),
            "--name",
            id,
        ],
    );
    assert!(output.status.success(), "{output:?}");
}

struct Join {
    child: Child,
    home: std::path::PathBuf,
    code: String,
}

impl Drop for Join {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `ws device join` in its own state directory and reads the code it prints.
fn join(dir: &Path, name: &str, addr: &str) -> Join {
    let home = dir.join(format!("join-{name}"));
    fs::create_dir_all(&home).unwrap();
    let mut command = ws(&home, &home.join("wsd.sock"));
    let mut child = command
        .args([
            "device",
            "join",
            &format!("https://{addr}"),
            "--name",
            name,
            "--ca",
        ])
        .arg(dir.join("root.pem"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let code = loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "no pairing code");
        if let Some(code) = line.trim().strip_prefix("Pairing code: ") {
            break code.to_owned();
        }
    };
    // Keep the pipe open so the child does not get SIGPIPE on its next line.
    std::mem::forget(stdout);
    Join { child, home, code }
}

fn approve(dir: &Path, code: &str) -> std::process::Output {
    let key_cmd = format!("cat '{}'", dir.join("root.key").display());
    cli(
        dir,
        &["device", "approve", code, "--root-key-cmd", &key_cmd],
    )
}

fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    for _ in 0..200 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the join command did not end");
}

#[test]
fn join_and_approve_pair_a_new_device() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    config_hub(dir, &hub.addr, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    assert_eq!(joining.code.len(), 8);
    let approved = approve(dir, &joining.code.to_lowercase());
    assert!(approved.status.success(), "{approved:?}");
    assert!(wait_exit(&mut joining.child).success());
    let state = joining.home.join("state");
    assert!(state.join("device/cert.pem").exists());
    let config = fs::read_to_string(state.join("config.toml")).unwrap();
    assert!(
        config.contains("[hub]") && config.contains(&hub.addr),
        "{config}"
    );
    // The new device calls the hub with its own certificate.
    let mut listed = ws(&joining.home, &joining.home.join("wsd.sock"));
    let listed = listed.args(["device", "list", "--json"]).output().unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let mut names: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["dev-a", "mini"]);
}

#[test]
fn used_and_unknown_codes_give_404() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    config_hub(dir, &hub.addr, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    assert!(approve(dir, &joining.code).status.success());
    assert!(wait_exit(&mut joining.child).success());
    for code in [joining.code.as_str(), "ZZZZZZZZ"] {
        let again = approve(dir, code);
        assert!(!again.status.success());
        assert!(
            String::from_utf8_lossy(&again.stderr).contains("404"),
            "{again:?}"
        );
    }
}

#[test]
fn approve_from_an_unregistered_device_gives_403_and_keeps_the_code() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    let mut joining = join(dir, "mini", &hub.addr);
    config_hub(dir, &hub.addr, "dev-b");
    let denied = approve(dir, &joining.code);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("403"),
        "{denied:?}"
    );
    assert!(joining.child.try_wait().unwrap().is_none());
    config_hub(dir, &hub.addr, "dev-a");
    assert!(approve(dir, &joining.code).status.success());
    assert!(wait_exit(&mut joining.child).success());
}

struct SyncedDevice {
    home: PathBuf,
    socket: PathBuf,
    _daemon: Daemon,
}

impl SyncedDevice {
    fn start(dir: &Path, hub_addr: &str, id: &str) -> Self {
        let home = dir.join(format!("home-{id}"));
        fs::create_dir_all(&home).unwrap();
        let socket = home.join("wsd.sock");
        config_hub_with(&home, dir, hub_addr, id);
        let daemon = Daemon::start(ws(&home, &socket), &socket);
        Self {
            home,
            socket,
            _daemon: daemon,
        }
    }

    fn create_session(&self, name: &str) -> String {
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/sessions",
            &serde_json::json!({"name": name}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let body: serde_json::Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        body["id"].as_str().unwrap().to_owned()
    }

    /// Sends one hook callback to the daemon hook endpoint.
    fn hook(&self, session: &str, state: &str, event: &str, at_us: u64) {
        let callback = serde_json::json!({
            "callback_id": ws_core::ids::Ulid::new(), "session_id": session,
            "observed_at_us": at_us, "state": state, "pane": null, "tmux_socket": null,
            "tmux_path": null, "payload": {"hook_event_name": event},
        });
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/hooks/ingest",
            &callback.to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    }

    fn all_sessions(&self) -> Vec<serde_json::Value> {
        let mut command = ws(&self.home, &self.socket);
        let output = command
            .args(["sessions", "--all-devices", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Polls the hub view of this device until the remote session has `state`.
    fn wait_remote(&self, session: &str, state: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let found = self
                .all_sessions()
                .into_iter()
                .find(|row| row["id"] == session && row["state"] == state);
            if let Some(row) = found {
                return row;
            }
            assert!(Instant::now() < deadline, "The wait timed out.");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn each_device_sees_the_sessions_of_the_other_device_and_their_state_changes() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    add_device(dir, "dev-b");
    let a = SyncedDevice::start(dir, &hub.addr, "dev-a");
    let b = SyncedDevice::start(dir, &hub.addr, "dev-b");
    let (alpha, beta) = (a.create_session("alpha"), b.create_session("beta"));

    let seen = b.wait_remote(&alpha, "created");
    assert_eq!(seen["device_name"], "dev-a");
    assert_eq!(seen["name"], "alpha");
    assert_eq!(seen["remote"], true);
    assert!(seen.get("alias").is_none(), "{seen}");
    a.wait_remote(&beta, "created");

    // A permission wait blocks the session until the next turn starts.
    a.hook(&alpha, "waiting", "PermissionRequest", 1);
    b.wait_remote(&alpha, "blocked");
    a.hook(&alpha, "working", "UserPromptSubmit", 2);
    let unblocked = b.wait_remote(&alpha, "created");
    assert_eq!(unblocked["device_name"], "dev-a");

    // The other direction, and the local rows come first without the remote duplicates.
    b.hook(&beta, "waiting", "PermissionRequest", 3);
    a.wait_remote(&beta, "blocked");
    let rows = a.all_sessions();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["id"], alpha);
    assert!(rows[0].get("remote").is_none());
    assert_eq!(rows[1]["id"], beta);
}

#[test]
fn the_sessions_route_needs_a_registered_device() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a", "dev-b"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    let outsider = SyncedDevice::start(dir, &hub.addr, "dev-b");
    let denied = ws(&outsider.home, &outsider.socket)
        .args(["sessions", "--all-devices"])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("403"),
        "{denied:?}"
    );
}

const SECRET: &str = "sk-live-SECRET-1234";

fn token_create(dir: &Path, name: &str) -> String {
    let output = cli(dir, &["hub", "token", "create", "--name", name]);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// One `POST /mcp` request with no client certificate. `token` None sends no header.
fn mcp(dir: &Path, addr: &str, token: Option<&str>, body: serde_json::Value) -> (u16, String) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = ws_hub::client::HubClient::anonymous(
        &format!("https://{addr}"),
        Some(&dir.join("root.pem")),
    )
    .unwrap();
    let authorization = token.map(|token| format!("Bearer {token}"));
    let headers: Vec<(&str, &str)> = authorization
        .iter()
        .map(|value| ("authorization", value.as_str()))
        .collect();
    let response = runtime
        .block_on(client.request_with_headers(
            hyper::Method::POST,
            "/mcp",
            &headers,
            &body.to_string(),
        ))
        .unwrap();
    (response.status, response.body)
}

fn tool(
    dir: &Path,
    addr: &str,
    token: &str,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    });
    let (status, body) = mcp(dir, addr, Some(token), call);
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn tool_json(result: &serde_json::Value) -> serde_json::Value {
    assert_eq!(result["result"]["isError"], false, "{result}");
    serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn mcp_tools_read_sessions_and_timelines_with_a_token_and_leak_no_secret() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    let a = SyncedDevice::start(dir, &hub.addr, "dev-a");
    let session = a.create_session("alpha");
    let log = ws_core::log::SessionLog::new(
        &a.home.join("state"),
        session.parse().unwrap(),
        ws_core::ids::Ulid::nil(),
    );
    let mut writer = log.writer();
    writer
        .append_current(
            "message.sent",
            serde_json::json!({"body": SECRET, "command": SECRET}),
        )
        .unwrap();
    writer
        .append_current("turn.ended", serde_json::json!({"summary": SECRET}))
        .unwrap();
    a.wait_remote(&session, "created");
    config_hub(dir, &hub.addr, "dev-a");
    let token = token_create(dir, "claude");

    let init = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    let (status, body) = mcp(dir, &hub.addr, Some(&token), init);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"tools\""), "{body}");
    let notice = serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(mcp(dir, &hub.addr, Some(&token), notice).0, 202);
    let listed = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    let (_, body) = mcp(dir, &hub.addr, Some(&token), listed);
    for name in ["list_sessions", "get_session", "get_session_timeline"] {
        assert!(body.contains(name), "{body}");
    }

    let mut transcript = String::new();
    let sessions = tool(
        dir,
        &hub.addr,
        &token,
        "list_sessions",
        serde_json::json!({}),
    );
    transcript += &sessions.to_string();
    let rows = tool_json(&sessions);
    assert_eq!(rows[0]["id"], session.as_str());
    assert_eq!(rows[0]["device_name"], "dev-a");
    let one = tool(
        dir,
        &hub.addr,
        &token,
        "get_session",
        serde_json::json!({"id": session}),
    );
    transcript += &one.to_string();
    assert_eq!(tool_json(&one)["name"], "alpha");
    let deadline = Instant::now() + Duration::from_secs(60);
    let timeline = loop {
        let timeline = tool(
            dir,
            &hub.addr,
            &token,
            "get_session_timeline",
            serde_json::json!({"id": session}),
        );
        if timeline.to_string().contains("turn.ended") {
            break timeline;
        }
        assert!(Instant::now() < deadline, "The wait timed out.");
        std::thread::sleep(Duration::from_millis(50));
    };
    transcript += &timeline.to_string();
    let entries = tool_json(&timeline)["entries"].as_array().unwrap().clone();
    assert!(!entries.is_empty(), "{timeline}");
    assert!(
        entries
            .iter()
            .any(|entry| entry["type"] == "session.created")
    );
    let missing = tool(
        dir,
        &hub.addr,
        &token,
        "get_session",
        serde_json::json!({"id": "nope"}),
    );
    assert_eq!(missing["result"]["isError"], true);
    assert!(!transcript.contains(SECRET), "{transcript}");

    let audit = cli(dir, &["hub", "audit", "--json"]);
    assert!(audit.status.success(), "{audit:?}");
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&audit.stdout).unwrap();
    let calls: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row["token"].as_str().unwrap(),
                row["tool"].as_str().unwrap(),
                row["result"].as_str().unwrap(),
            )
        })
        .collect();
    // The timeline poll adds one or more `get_session_timeline` rows.
    let (last, rest) = calls.split_last().unwrap();
    assert_eq!(
        calls[..2],
        [
            ("claude", "list_sessions", "ok"),
            ("claude", "get_session", "ok")
        ]
    );
    assert_eq!(*last, ("claude", "get_session", "error"));
    assert!(
        rest[2..]
            .iter()
            .all(|call| *call == ("claude", "get_session_timeline", "ok"))
    );
    assert!(rest.len() >= 3, "{calls:?}");
    assert!(rows.iter().all(|row| row["ts_ms"].as_i64().unwrap() > 0));
    let text = cli(dir, &["hub", "audit"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("claude\tlist_sessions\tok"));
}

#[test]
fn missing_unknown_and_revoked_tokens_give_401_and_the_list_shows_no_token() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    let dir = sandbox.path();
    pki(dir, &["dev-a"]);
    let hub = serve(dir);
    add_device(dir, "dev-a");
    config_hub(dir, &hub.addr, "dev-a");
    let token = token_create(dir, "claude");
    let ping = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    assert_eq!(mcp(dir, &hub.addr, Some(&token), ping.clone()).0, 200);
    assert_eq!(mcp(dir, &hub.addr, None, ping.clone()).0, 401);
    assert_eq!(
        mcp(dir, &hub.addr, Some("wsmcp_unknown"), ping.clone()).0,
        401
    );

    let listed = cli(dir, &["hub", "token", "list", "--json"]);
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("claude") && text.contains("read") && !text.contains(&token));
    let revoked = cli(dir, &["hub", "token", "revoke", "claude"]);
    assert!(revoked.status.success(), "{revoked:?}");
    assert_eq!(mcp(dir, &hub.addr, Some(&token), ping).0, 401);
    let plain = cli(dir, &["hub", "token", "list"]);
    assert!(String::from_utf8_lossy(&plain.stdout).contains("claude\tread\trevoked"));
    let again = cli(dir, &["hub", "token", "revoke", "claude"]);
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("404"),
        "{again:?}"
    );
}

#[test]
fn token_and_audit_commands_need_the_hub_section() {
    let sandbox = tempfile::tempdir_in("/tmp").unwrap();
    for arguments in [
        vec!["hub", "token", "create", "--name", "x"],
        vec!["hub", "token", "list"],
        vec!["hub", "token", "revoke", "x"],
        vec!["hub", "audit"],
    ] {
        let output = cli(sandbox.path(), &arguments);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
}
