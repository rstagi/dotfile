use std::{collections::BTreeMap, fs, path::Path, process::Command};

use serde_json::json;
use ws_daemon::sessions::{CreateSession, Sessions};

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
