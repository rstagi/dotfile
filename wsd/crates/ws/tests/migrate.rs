use std::{collections::BTreeMap, fs, path::Path, process::Command};

use serde_json::json;
use ws_core::{device::Device, log::SessionLog};
use ws_daemon::sessions::{CreateSession, Sessions};

struct LockHolder(std::process::Child);

#[test]
fn respects_zsh_fcntl_locks_before_reading_the_snapshot() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    fs::create_dir_all(source.join("sessions/1")).unwrap();
    fs::write(source.join("sessions/counter"), "1\n").unwrap();
    fs::write(
        source.join("sessions/1/meta"),
        json!({"id":"1", "status":"active"}).to_string(),
    )
    .unwrap();
    for name in [
        "counter", "names", "1/output", "1/inbox", "1/meta", "1/events",
    ] {
        let path = source.join(format!("sessions/{name}.lock"));
        fs::write(&path, "").unwrap();
        let _holder = LockHolder::new(&path);
        let output = migrate(root.path(), &source, &state);
        assert!(
            !output.status.success(),
            "The import ignored {name}: {output:?}"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("zsh state is busy"), "{error}");
        assert!(error.contains(&format!("{name}.lock")), "{error}");
        assert!(!state.exists());
    }
    let output = migrate(root.path(), &source, &state);
    assert!(output.status.success(), "{output:?}");
}

impl LockHolder {
    fn new(path: &Path) -> Self {
        use std::{
            io::{BufRead, BufReader},
            process::Stdio,
            time::Duration,
        };
        let mut child = Command::new("zsh").args(["-fc", "zmodload zsh/system; zsystem flock -t 0 \"$1\" || exit 1; print ready; read -r release", "lock-holder"])
            .arg(path).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let holder = Self(child);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).unwrap();
            let _ = sender.send(line);
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(30)).unwrap(),
            "ready\n"
        );
        holder
    }
}

impl Drop for LockHolder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn imports_all_artifacts_and_requeues_interrupted_stop_and_unconfirmed_paste() {
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    let capture = Command::new("zsh")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ws-daemon/src/import/fixtures/capture.zsh"
        ))
        .arg(&source)
        .env("HOME", root.path())
        .output()
        .unwrap();
    assert!(capture.status.success(), "{capture:?}");
    let before = tree(&source);
    let output = migrate(root.path(), &source, &state);
    assert!(output.status.success(), "{output:?}");
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let done = sessions.get("#1", None).unwrap();
    let target = sessions.get("#2", None).unwrap();
    let human = sessions.get("human", None).unwrap();
    assert_eq!(done.state, "finished");
    assert_eq!(
        fs::read(
            state
                .join("sessions")
                .join(done.id.to_string())
                .join("output.md")
        )
        .unwrap(),
        b"completed output"
    );
    assert_eq!(
        fs::read(
            state
                .join("sessions")
                .join(target.id.to_string())
                .join("prompt")
        )
        .unwrap(),
        before["sessions/2/prompt"]
    );
    let entries = SessionLog::new(&state, done.id, Device::load(&state).unwrap().device_id)
        .read()
        .unwrap()
        .entries;
    let mapping = plan["sessions"][0]["event_map"].as_array().unwrap();
    assert_eq!(mapping.len(), 2);
    for (index, event_type) in ["done", "exit"].iter().enumerate() {
        let entry = entries
            .iter()
            .find(|entry| entry.seq == mapping[index]["seq"].as_u64().unwrap())
            .unwrap();
        assert_eq!(&entry.event_type, event_type);
        assert_eq!(mapping[index]["line"], index + 1);
        let original = std::str::from_utf8(&before["sessions/1/events"])
            .unwrap()
            .lines()
            .nth(index)
            .unwrap();
        assert_eq!(mapping[index]["raw"], original);
    }
    let messages = sessions.inbox(target.id).unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.content.body.as_str())
            .collect::<Vec<_>>(),
        [
            "[ws terminal]\nunconfirmed paste",
            "[ws #1]\ninterrupted Stop reply",
            "[ws terminal]\nqueued message"
        ]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.content.from_session == human.id)
    );
    let device = Device::load(&state).unwrap().device_id;
    let sent = SessionLog::new(&state, human.id, device)
        .read()
        .unwrap()
        .entries;
    assert_eq!(
        sent.iter()
            .filter(|event| event.event_type == "message.sent")
            .count(),
        3
    );
    assert!(
        !sent
            .iter()
            .any(|event| event.event_type == "message.delivered")
    );
    // Import may create lock files. Every pre-existing source artifact must stay unchanged.
    let after = tree(&source);
    for (path, bytes) in before {
        assert_eq!(after[&path], bytes, "{path}");
    }
    drop(sessions);
    let imported = tree(&state);
    let repeated = migrate(root.path(), &source, &state);
    assert!(repeated.status.success(), "{repeated:?}");
    assert_eq!(repeated.stdout, output.stdout);
    assert_eq!(tree(&state), imported);
}

#[test]
fn imports_identity_and_keeps_the_alias_floor_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    fs::create_dir_all(source.join("sessions/7")).unwrap();
    fs::write(source.join("sessions/counter"), "12\n").unwrap();
    fs::write(source.join("sessions/7/meta"), json!({"id":"7", "name":"worker", "agent":"codex", "path":"/repo", "workspace":"repo/main", "status":"closed"}).to_string()).unwrap();
    let output = migrate(root.path(), &source, &state);
    assert!(output.status.success(), "{output:?}");
    let before = tree(&state);
    let repeated = migrate(root.path(), &source, &state);
    assert!(repeated.status.success(), "{repeated:?}");
    assert_eq!(repeated.stdout, output.stdout);
    assert_eq!(tree(&state), before);
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let imported = sessions.get("#7", None).unwrap();
    assert_eq!(imported.name.as_deref(), Some("worker"));
    assert_eq!(imported.state, "failed");
    assert_eq!(imported.body["agent"], "codex");
    assert_eq!(imported.body["path"], "/repo");
    assert_eq!(sessions.create(CreateSession::default()).unwrap().alias, 13);
}

#[test]
fn dry_run_reports_the_import_without_writing_source_or_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("legacy");
    let state = root.path().join("state");
    fs::create_dir_all(source.join("sessions/7")).unwrap();
    fs::write(source.join("sessions/counter"), "12\n").unwrap();
    fs::write(source.join("sessions/7/meta"), json!({"id":"7", "name":"worker", "agent":"codex", "path":"/repo", "workspace":"repo/main", "status":"closed"}).to_string()).unwrap();
    let before = tree(&source);
    let output = Command::new(env!("CARGO_BIN_EXE_ws"))
        .args(["migrate", "--from-zsh", "--dry-run"])
        .env("HOME", root.path())
        .env("WS_HOME", &source)
        .env("WS_STATE", &state)
        .env("WS_SOCK", root.path().join("absent.sock"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["alias_floor"], 13);
    assert_eq!(plan["sessions"][0]["alias"], 7);
    assert_eq!(tree(&source), before);
    assert!(!state.exists());
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .display()
                        .to_string(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn migrate(home: &Path, source: &Path, state: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ws"))
        .args(["migrate", "--from-zsh"])
        .env("HOME", home)
        .env("WS_HOME", source)
        .env("WS_STATE", state)
        .env("WS_SOCK", home.join("absent.sock"))
        .output()
        .unwrap()
}
