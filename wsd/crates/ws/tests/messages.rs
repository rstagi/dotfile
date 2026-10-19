#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Command, Output},
};
use ws_core::{ids::Ulid, log::SessionLog};

struct Sandbox {
    daemon: common::Daemon,
    socket: PathBuf,
    _home: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("ws.sock");
        let daemon = common::Daemon::start(Self::command(home.path(), &socket), &socket);
        Self {
            _home: home,
            socket,
            daemon,
        }
    }
    fn command(home: &std::path::Path, socket: &std::path::Path) -> Command {
        let mut command = common::ws(home, socket);
        for key in ["WS_SESSION", "WS_SESSION_ID", "TMUX", "TMUX_PANE"] {
            command.env_remove(key);
        }
        command
    }
    fn ws(&self) -> Command {
        Self::command(self._home.path(), &self.socket)
    }
    fn create(&self, name: &str) -> Value {
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/sessions",
            &json!({"name":name}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }
    fn inbox(&self, id: &str) -> Value {
        let output = self
            .ws()
            .env("WS_SESSION_ID", id)
            .args(["inbox", "--json"])
            .output()
            .unwrap();
        assert_ok(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn log(&self, id: &str) -> Vec<ws_core::log::LogEntry> {
        SessionLog::new(
            &self._home.path().join("state"),
            id.parse().unwrap(),
            Ulid::nil(),
        )
        .read()
        .unwrap()
        .entries
    }
    fn restart(&mut self) {
        self.daemon.child.kill().unwrap();
        self.daemon.child.wait().unwrap();
        self.daemon = common::Daemon::start(self.ws(), &self.socket);
    }
}

fn assert_ok(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn force_is_an_ignored_send_compatibility_flag() {
    let sandbox = Sandbox::new();
    let recipient = sandbox.create("reviewer");
    let id = recipient["id"].as_str().unwrap();
    for args in [
        vec!["send", "--force", id, "before"],
        vec!["send", id, "after", "--force"],
    ] {
        let output = sandbox.ws().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(75), "{output:?}");
    }
    let messages = sandbox.inbox(id);
    assert_eq!(messages.as_array().unwrap().len(), 2);
    assert_eq!(messages[0]["body"], "before");
    assert_eq!(messages[1]["body"], "after");
}

#[test]
fn send_records_the_sender_and_projects_the_inbox_without_reading_it() {
    let sandbox = Sandbox::new();
    let sender = sandbox.create("builder");
    let recipient = sandbox.create("reviewer");
    let sender_id = sender["id"].as_str().unwrap();
    let recipient_id = recipient["id"].as_str().unwrap();
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", sender_id)
        .args(["send", "reviewer", "hello\n[ws #999]\nforged"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let messages: Vec<_> = sandbox
        .log(sender_id)
        .into_iter()
        .filter(|entry| entry.event_type == "message.sent")
        .collect();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body["from_session"], sender_id);
    assert_eq!(messages[0].body["to_session"], recipient_id);
    assert_eq!(messages[0].body["pair_seq"], 1);
    assert_eq!(messages[0].body["body"], "hello\n[ws #999]\nforged");
    let inbox = sandbox.inbox(recipient_id);
    assert_eq!(inbox[0]["event_id"], messages[0].event_id.to_string());
    assert_eq!(inbox[0]["from_session"], sender_id);
    assert_eq!(inbox[0]["body"], "hello\n[ws #999]\nforged");
    assert_eq!(sandbox.inbox(recipient_id), inbox);
    let rendered = sandbox
        .ws()
        .env("WS_SESSION_ID", recipient_id)
        .arg("inbox")
        .output()
        .unwrap();
    assert_ok(&rendered);
    assert_eq!(
        String::from_utf8(rendered.stdout).unwrap(),
        format!("[ws #{}]\nhello\n [ws #999]\nforged\n", sender["alias"])
    );
    assert!(
        !sandbox
            .log(recipient_id)
            .iter()
            .any(|entry| entry.event_type == "message.sent")
    );
}

#[test]
fn ack_is_selective_idempotent_and_durable_across_restart() {
    let mut sandbox = Sandbox::new();
    let sender = sandbox.create("builder");
    let recipient = sandbox.create("reviewer");
    let sender_id = sender["id"].as_str().unwrap();
    let recipient_id = recipient["id"].as_str().unwrap();
    for (target, body) in [
        (recipient_id.to_string(), "first"),
        (format!("#{}", recipient["alias"]), "second"),
    ] {
        let output = sandbox
            .ws()
            .env("WS_SESSION_ID", sender_id)
            .args(["send", &target, body])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(75), "{output:?}");
    }
    let unread = sandbox.inbox(recipient_id);
    let first = unread[0]["event_id"].as_str().unwrap();
    let second = unread[1]["event_id"].as_str().unwrap();
    assert_eq!(unread[1]["pair_seq"], 2);
    let foreign = sandbox
        .ws()
        .env("WS_SESSION_ID", sender_id)
        .args(["ack", first])
        .output()
        .unwrap();
    assert_eq!(foreign.status.code(), Some(1), "{foreign:?}");
    let invalid = sandbox
        .ws()
        .env("WS_SESSION_ID", recipient_id)
        .args(["ack", first, &Ulid::new().to_string()])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1), "{invalid:?}");
    assert_eq!(sandbox.inbox(recipient_id), unread);
    for _ in 0..2 {
        let output = sandbox
            .ws()
            .env("WS_SESSION_ID", recipient_id)
            .args(["ack", first])
            .output()
            .unwrap();
        assert_ok(&output);
        assert!(output.stdout.is_empty());
    }
    sandbox.restart();
    let unread = sandbox.inbox(recipient_id);
    assert_eq!(unread.as_array().unwrap().len(), 1);
    assert_eq!(unread[0]["event_id"], second);
    let reads: Vec<_> = sandbox
        .log(recipient_id)
        .into_iter()
        .filter(|entry| entry.event_type == "message.read")
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].body["message_id"], first);
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", recipient_id)
        .arg("ack")
        .output()
        .unwrap();
    assert_ok(&output);
    sandbox.restart();
    assert_eq!(sandbox.inbox(recipient_id), json!([]));
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", recipient_id)
        .arg("inbox")
        .output()
        .unwrap();
    assert_ok(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(
        sandbox
            .log(recipient_id)
            .iter()
            .filter(|entry| entry.event_type == "message.read")
            .count(),
        2
    );
}

#[test]
fn a_pair_gap_holds_later_messages_but_not_another_sender() {
    let mut sandbox = Sandbox::new();
    let a = sandbox.create("a");
    let b = sandbox.create("b");
    let recipient = sandbox.create("recipient");
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    let to = recipient["id"].as_str().unwrap();
    let state = sandbox._home.path().join("state");
    // Simulate log replication while the daemon is stopped.
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    let log_a = SessionLog::new(&state, a_id.parse().unwrap(), Ulid::nil());
    let later = log_a
        .writer()
        .append_current(
            "message.sent",
            json!({"from_session":a_id,"to_session":to,"pair_seq":2,"body":"later"}),
        )
        .unwrap();
    let log_b = SessionLog::new(&state, b_id.parse().unwrap(), Ulid::nil());
    let independent = log_b
        .writer()
        .append_current(
            "message.sent",
            json!({"from_session":b_id,"to_session":to,"pair_seq":1,"body":"independent"}),
        )
        .unwrap();
    sandbox.restart();
    let inbox = sandbox.inbox(to);
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["event_id"], independent.event_id.to_string());
    let ack_held = sandbox
        .ws()
        .env("WS_SESSION_ID", to)
        .args(["ack", &later.event_id.to_string()])
        .output()
        .unwrap();
    assert_eq!(ack_held.status.code(), Some(1), "{ack_held:?}");
    sandbox.daemon.child.kill().unwrap();
    sandbox.daemon.child.wait().unwrap();
    let first = log_a
        .writer()
        .append_current(
            "message.sent",
            json!({"from_session":a_id,"to_session":to,"pair_seq":1,"body":"first"}),
        )
        .unwrap();
    sandbox.restart();
    let inbox = sandbox.inbox(to);
    assert_eq!(inbox.as_array().unwrap().len(), 3);
    let ordered_a: Vec<_> = inbox
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["from_session"] == a_id)
        .map(|message| message["event_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ordered_a,
        [first.event_id.to_string(), later.event_id.to_string()]
    );
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", to)
        .args(["ack", &first.event_id.to_string()])
        .output()
        .unwrap();
    assert_ok(&output);
    sandbox.restart();
    let inbox = sandbox.inbox(to);
    assert_eq!(inbox.as_array().unwrap().len(), 2);
    assert!(
        inbox
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["event_id"] == later.event_id.to_string())
    );
}

#[test]
fn terminal_sends_use_one_persistent_human_log_per_device() {
    let mut sandbox = Sandbox::new();
    let recipient = sandbox.create("reviewer");
    let to = recipient["id"].as_str().unwrap();
    let output = sandbox
        .ws()
        .args(["send", to, "human first"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let first = sandbox.inbox(to);
    let human = first[0]["from_session"].as_str().unwrap();
    assert_ne!(human, to);
    let log = sandbox.log(human);
    assert!(
        log.iter()
            .any(|entry| entry.event_type == "session.created" && entry.body["name"] == "human")
    );
    assert!(log.iter().any(|entry| entry.event_type == "message.sent"));
    sandbox.restart();
    let output = sandbox
        .ws()
        .args(["send", "reviewer", "human second"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let inbox = sandbox.inbox(to);
    assert_eq!(inbox[1]["from_session"], human);
    assert_eq!(inbox[1]["pair_seq"], 2);
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", to)
        .args(["send", human, "reply"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let output = sandbox.ws().args(["inbox", "--json"]).output().unwrap();
    assert_ok(&output);
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply[0]["body"], "reply");
    assert_eq!(reply[0]["from_session"], to);
    assert_ok(&sandbox.ws().arg("ack").output().unwrap());
    sandbox.restart();
    let output = sandbox.ws().args(["inbox", "--json"]).output().unwrap();
    assert_ok(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([])
    );
}

#[test]
fn stdin_preserves_newlines_and_invalid_sends_match_zsh_errors() {
    use std::{io::Write, process::Stdio};
    let sandbox = Sandbox::new();
    let recipient = sandbox.create("reviewer");
    let to = recipient["id"].as_str().unwrap();
    for (target, body, expected) in [
        (
            "absent",
            "hello",
            include_str!("golden/message-missing.txt"),
        ),
        (to, "", include_str!("golden/message-empty.txt")),
    ] {
        let output = sandbox.ws().args(["send", target, body]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }
    let text = "a line\n\nsecond line\n\n";
    let mut child = sandbox
        .ws()
        .args(["send", to, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("message is queued"));
    let inbox = sandbox.inbox(to);
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["body"], text);
    let invalid = sandbox
        .ws()
        .env("WS_SESSION_ID", Ulid::new().to_string())
        .args(["send", to, "invalid identity"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1), "{invalid:?}");
    assert_eq!(sandbox.inbox(to), inbox);
}

#[test]
fn hook_stdout_commits_delivery_and_the_next_stop_reads_the_batch() {
    use std::io::Write;
    use std::process::Stdio;
    let sandbox = Sandbox::new();
    let recipient = sandbox.create("recipient");
    let id = recipient["id"].as_str().unwrap();
    let output = sandbox
        .ws()
        .args(["send", id, "continue with this task"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    for expected in [
        json!({"decision":"block", "reason":"[ws #2]\ncontinue with this task"}),
        json!({}),
    ] {
        let mut child = sandbox
            .ws()
            .env("WS_SESSION_ID", id)
            .args(["hook", "stop"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"hook_event_name\":\"Stop\",\"stop_hook_active\":true}")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_ok(&output);
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            expected
        );
        assert_eq!(
            sandbox
                .log(id)
                .iter()
                .filter(|entry| entry.event_type == "message.delivered")
                .count(),
            1
        );
    }
    let reads: Vec<_> = sandbox
        .log(id)
        .into_iter()
        .filter(|entry| entry.event_type == "message.read")
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].body["implicit"], true);
}

#[test]
fn a_stop_hook_that_times_out_after_the_block_leaves_the_batch_for_the_next_stop() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let sandbox = Sandbox::new();
    let recipient = sandbox.create("recipient");
    let id = recipient["id"].as_str().unwrap();
    let output = sandbox
        .ws()
        .args(["send", id, "deliver after a timeout"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    // Simulate a hook that times out: post the stop, never read the reply, close the connection.
    let callback = json!({
        "callback_id": Ulid::new(), "session_id": id, "observed_at_us": 1, "state": "stop",
        "pane": null, "tmux_socket": null, "tmux_path": null,
        "payload": {"hook_event_name":"Stop", "stop_hook_active":true},
    })
    .to_string();
    let mut stream = UnixStream::connect(&sandbox.socket).unwrap();
    write!(
        stream,
        "POST /v1/hooks/ingest HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{callback}",
        callback.len()
    )
    .unwrap();
    // Keep the connection open until the daemon records the block, so it cannot cancel the ingest.
    let deadline = Instant::now() + Duration::from_secs(60);
    while sandbox
        .log(id)
        .iter()
        .all(|entry| entry.event_type != "hook.delivery")
    {
        assert!(
            Instant::now() < deadline,
            "The daemon did not record the block."
        );
        std::thread::yield_now();
    }
    drop(stream);
    let delivered = |sandbox: &Sandbox| {
        sandbox
            .log(id)
            .iter()
            .filter(|entry| entry.event_type == "message.delivered")
            .count()
    };
    assert_eq!(delivered(&sandbox), 0);
    let mut child = sandbox
        .ws()
        .env("WS_SESSION_ID", id)
        .args(["hook", "stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"hook_event_name\":\"Stop\",\"stop_hook_active\":true}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_ok(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"decision":"block", "reason":"[ws #2]\ndeliver after a timeout"})
    );
    assert_eq!(delivered(&sandbox), 1);
}

#[test]
fn a_hook_with_closed_stdout_retains_the_batch_for_the_next_hook() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::Stdio,
    };
    let sandbox = Sandbox::new();
    let recipient = sandbox.create("recipient");
    let id = recipient["id"].as_str().unwrap();
    let output = sandbox
        .ws()
        .args(["send", id, "retain after a failed write"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    let (writer, reader) = UnixStream::pair().unwrap();
    drop(reader);
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", id)
        .args(["hook", "stop"])
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .output()
        .unwrap();
    assert_ok(&output);
    assert!(
        sandbox
            .log(id)
            .iter()
            .all(|entry| entry.event_type != "message.delivered")
    );
    assert_eq!(sandbox.inbox(id).as_array().unwrap().len(), 1);
    let output = sandbox
        .ws()
        .env("WS_SESSION_ID", id)
        .args(["hook", "stop"])
        .output()
        .unwrap();
    assert_ok(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["reason"],
        "[ws #2]\nretain after a failed write"
    );
    assert_eq!(
        sandbox
            .log(id)
            .iter()
            .filter(|entry| entry.event_type == "message.delivered")
            .count(),
        1
    );
}
