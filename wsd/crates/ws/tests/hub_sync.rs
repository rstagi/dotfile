//! Session log sync: two sandbox daemons push to one test hub. Each wait polls the hub store
//! (the hub cursor) or the daemon socket. No wait is a fixed sleep.

use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use ws_core::{ids::Ulid, log::SessionLog};
use ws_hub::{
    client::HubClient,
    sync::{ExportEntry, SessionBatch, StoredEntry, SyncBatch, SyncStore},
};

mod common;
use common::{
    Daemon,
    hub_fixture::{Hub, config_hub_with, pki, serve, serve_on},
    ws,
};

const SECRET: &str = "sk-live-SECRET-1234";

struct Device {
    home: PathBuf,
    socket: PathBuf,
    daemon: Daemon,
}

struct Fixture {
    dir: tempfile::TempDir,
    store: SyncStore,
    hub: Hub,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        pki(dir.path(), &["dev-a", "dev-b"]);
        let hub = serve(dir.path());
        for id in ["dev-a", "dev-b"] {
            let output = ws(dir.path(), &dir.path().join("x.sock"))
                .args(["hub", "add-device", "--state"])
                .arg(dir.path().join("hub"))
                .arg("--cert")
                .arg(dir.path().join(format!("{id}.pem")))
                .args(["--name", id])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        let store = SyncStore::open(&dir.path().join("hub")).unwrap();
        Self { dir, store, hub }
    }

    fn device(&self, id: &'static str) -> Device {
        let home = self.dir.path().join(format!("home-{id}"));
        fs::create_dir_all(&home).unwrap();
        let socket = home.join("wsd.sock");
        config_hub_with(&home, self.dir.path(), &self.hub.addr, id);
        let daemon = Daemon::start(ws(&home, &socket), &socket);
        Device {
            home,
            socket,
            daemon,
        }
    }

    fn client(&self, id: &str) -> HubClient {
        let dir = self.dir.path();
        HubClient::new(
            &format!("https://{}", self.hub.addr),
            &dir.join(format!("{id}.pem")),
            &dir.join(format!("{id}.key")),
            &dir.join("root.pem"),
        )
        .unwrap()
    }

    fn entries(&self, device: &str) -> Vec<StoredEntry> {
        self.store.entries(device).unwrap()
    }

    /// Waits until the hub cursor of the session reaches `seq`.
    fn wait_cursor(&self, device: &str, session: &str, seq: u64) {
        wait(|| {
            self.store
                .cursors(device)
                .unwrap()
                .get(session)
                .is_some_and(|&c| c >= seq)
        });
    }
}

impl Device {
    fn state(&self) -> PathBuf {
        self.home.join("state")
    }

    fn create_session(&self, name: &str) -> String {
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/sessions",
            &json!({"name": name}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let body: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        body["id"].as_str().unwrap().to_owned()
    }

    /// Appends an entry to the log of a session and returns its sequence number.
    fn append(&self, session: &str, event_type: &str, body: Value) -> u64 {
        SessionLog::new(&self.state(), session.parse().unwrap(), Ulid::nil())
            .writer()
            .append_current(event_type, body)
            .unwrap()
            .seq
    }

    fn restart(&mut self) {
        self.daemon.child.kill().unwrap();
        self.daemon.child.wait().unwrap();
        self.daemon = Daemon::start(ws(&self.home, &self.socket), &self.socket);
    }
}

fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(Instant::now() < deadline, "The wait timed out.");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn event_ids(entries: &[StoredEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|stored| stored.entry.event_id.clone())
        .collect()
}

fn assert_unique(entries: &[StoredEntry]) {
    let ids = event_ids(entries);
    assert_eq!(
        ids.len(),
        ids.iter().collect::<HashSet<_>>().len(),
        "{ids:?}"
    );
}

#[test]
fn two_daemons_push_new_entries_with_the_device_of_the_certificate() {
    let fixture = Fixture::new();
    let a = fixture.device("dev-a");
    let b = fixture.device("dev-b");
    let (sa, sb) = (a.create_session("alpha"), b.create_session("beta"));
    let seq_a = a.append(&sa, "turn.started", json!({}));
    let seq_b = b.append(&sb, "turn.started", json!({}));
    fixture.wait_cursor("dev-a", &sa, seq_a);
    fixture.wait_cursor("dev-b", &sb, seq_b);
    for (device, session) in [("dev-a", &sa), ("dev-b", &sb)] {
        let entries = fixture.entries(device);
        assert!(
            entries
                .iter()
                .all(|stored| &stored.entry.session_id == session),
            "{entries:?}"
        );
        assert_eq!(entries.first().unwrap().entry.event_type, "session.created");
        assert_eq!(entries.last().unwrap().entry.event_type, "turn.started");
    }
}

#[test]
fn a_duplicate_push_stores_one_entry_and_the_body_device_id_is_ignored() {
    let fixture = Fixture::new();
    let a = fixture.device("dev-a");
    let session = a.create_session("alpha");
    let seq = a.append(&session, "turn.ended", json!({}));
    fixture.wait_cursor("dev-a", &session, seq);
    let before = fixture.entries("dev-a");
    let fresh = Ulid::new().to_string();
    let mut entries: Vec<_> = before.iter().map(|stored| stored.entry.clone()).collect();
    entries.push(ExportEntry {
        event_id: fresh.clone(),
        session_id: session.clone(),
        seq: seq + 1,
        ts_ms: 1,
        event_type: "turn.started".into(),
        body: json!({}),
    });
    let mut body = serde_json::to_value(SyncBatch {
        sessions: vec![SessionBatch {
            session_id: session.clone(),
            through_seq: seq + 1,
            entries,
        }],
    })
    .unwrap();
    body["sessions"][0]["entries"][0]["device_id"] = json!("dev-b");
    let response = futures_block(
        fixture
            .client("dev-a")
            .post_json("/v1/sync/logs", body.to_string().into_bytes()),
    )
    .unwrap();
    assert_eq!(response.status, 200, "{}", response.body);
    let after = fixture.entries("dev-a");
    assert_eq!(after.len(), before.len() + 1);
    assert_unique(&after);
    assert!(fixture.entries("dev-b").is_empty());
    assert!(after.iter().any(|stored| stored.entry.event_id == fresh));
}

#[test]
fn entries_arrive_one_time_after_a_hub_restart_and_after_a_daemon_restart() {
    let mut fixture = Fixture::new();
    let mut a = fixture.device("dev-a");
    let session = a.create_session("alpha");
    let seq = a.append(&session, "turn.started", json!({}));
    fixture.wait_cursor("dev-a", &session, seq);

    fixture.restart_hub_down_while(|| {
        a.append(&session, "agent.waiting", json!({}));
        a.append(&session, "turn.ended", json!({}));
    });
    let seq = a.append(&session, "turn.started", json!({}));
    fixture.wait_cursor("dev-a", &session, seq);
    let entries = fixture.entries("dev-a");
    assert_unique(&entries);
    let types: Vec<_> = entries
        .iter()
        .map(|stored| stored.entry.event_type.as_str())
        .collect();
    assert_eq!(
        types,
        [
            "session.created",
            "turn.started",
            "agent.waiting",
            "turn.ended",
            "turn.started"
        ]
    );

    a.restart();
    let seq = a.append(&session, "turn.ended", json!({}));
    fixture.wait_cursor("dev-a", &session, seq);
    let entries = fixture.entries("dev-a");
    assert_unique(&entries);
    assert_eq!(entries.len(), 6);
}

#[test]
fn no_uploaded_entry_contains_a_secret_from_a_hook_a_tool_a_message_or_a_command() {
    let fixture = Fixture::new();
    let a = fixture.device("dev-a");
    let session = a.create_session("alpha");
    a.append(
        &session,
        "hook.received",
        json!({"payload": {"tool_input": {"command": SECRET}, "note": SECRET}}),
    );
    a.append(
        &session,
        "tool.result",
        json!({"result": SECRET, "tool_response": SECRET}),
    );
    a.append(
        &session,
        "message.sent",
        json!({"from_session": session, "to_session": session, "pair_seq": 1, "body": SECRET}),
    );
    a.append(
        &session,
        "process.started",
        json!({"command": SECRET, "argv": ["sh", "-c", SECRET]}),
    );
    let seq = a.append(&session, "turn.ended", json!({"summary": SECRET}));
    fixture.wait_cursor("dev-a", &session, seq);
    let entries = fixture.entries("dev-a");
    assert!(entries.len() >= 2, "{entries:?}");
    assert!(!serde_json::to_string(&entries).unwrap().contains(SECRET));
    let db = fs::read(fixture.dir.path().join("hub/hub.db")).unwrap();
    assert!(
        !db.windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes())
    );
}

#[test]
fn a_daemon_without_a_hub_section_starts_and_pushes_nothing() {
    let fixture = Fixture::new();
    let home = fixture.dir.path().join("home-none");
    fs::create_dir_all(&home).unwrap();
    let socket = home.join("wsd.sock");
    let _daemon = Daemon::start(ws(&home, &socket), &socket);
    let response = common::request(
        &socket,
        "POST",
        "/v1/sessions",
        &json!({"name": "x"}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert!(fixture.store.cursors("dev-a").unwrap().is_empty());
    assert!(fixture.entries("dev-a").is_empty());
}

impl Fixture {
    /// Stops the hub, runs `during` while the hub is down, then starts the hub on the same address.
    fn restart_hub_down_while(&mut self, during: impl FnOnce()) {
        let addr = self.hub.addr.clone();
        self.hub.child.kill().unwrap();
        self.hub.child.wait().unwrap();
        during();
        self.hub = serve_on(self.dir.path(), &addr);
    }
}

fn futures_block<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
