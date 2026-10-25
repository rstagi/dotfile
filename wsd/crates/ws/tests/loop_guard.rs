#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use ws_daemon::agents::Prepared;

struct Sandbox {
    _daemon: common::Daemon,
    home: tempfile::TempDir,
    socket: PathBuf,
}

#[test]
fn preparation_guards_loop_descendants_and_removes_inherited_loop_identity_elsewhere() {
    let sandbox = Sandbox::new();
    let owner = sandbox.loop_session();
    for agent in ["claude", "codex", "grok"] {
        let parent = sandbox.session(Some(&owner));
        let child = sandbox.session(Some(&parent));
        let plain = sandbox.session(None);
        for id in [&parent, &child] {
            let prepared = sandbox.prepare(id, agent);
            assert_eq!(prepared.env.get("WS_LOOP_ID"), Some(&owner));
            let all = format!("{:?} {:?}", prepared.argv, prepared.files.iter()
                .map(|file| String::from_utf8_lossy(&file.contents)).collect::<Vec<_>>());
            assert!(all.contains("hook guard"), "{agent}: {all}");
            if agent != "codex" {
                let file = prepared.files.iter().find(|file| {
                    serde_json::from_slice::<Value>(&file.contents).ok()
                        .is_some_and(|v| v["hooks"]["PreToolUse"].is_array())
                }).unwrap();
                let settings: Value = serde_json::from_slice(&file.contents).unwrap();
                let pre = settings["hooks"]["PreToolUse"].to_string();
                assert!(pre.contains("hook working"));
                assert!(pre.contains("hook guard"));
            }
        }
        let prepared = sandbox.prepare(&plain, agent);
        assert!(!prepared.env.contains_key("WS_LOOP_ID"));
        assert!(prepared.unset_env.iter().any(|key| key == "WS_LOOP_ID"));
        assert!(!format!("{:?}", prepared.argv).contains("hook guard"));
        assert!(prepared.files.iter().all(|file| !String::from_utf8_lossy(&file.contents).contains("hook guard")));
    }
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let mut command = common::ws(home.path(), &socket);
        command.env_remove("WS_LOOP_ID").env_remove("WS_LOOP_GUARD_AGENT");
        let daemon = common::Daemon::start(command, &socket);
        Self { _daemon: daemon, home, socket }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command.env_remove("WS_LOOP_ID").env_remove("WS_LOOP_GUARD_AGENT");
        command
    }

    fn post(&self, path: &str, body: Value) -> Value {
        let response = common::request(&self.socket, "POST", path, &body.to_string());
        assert!(response.starts_with("HTTP/1.1 20"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn loop_session(&self) -> String {
        let directory = self.home.path().join("loop");
        fs::create_dir(&directory).unwrap();
        self.post("/v1/loops/create", json!({"loop_dir":directory}))["id"]
            .as_str().unwrap().to_owned()
    }

    fn session(&self, parent: Option<&str>) -> String {
        self.post("/v1/sessions", json!({"parent_session_id":parent}))["id"]
            .as_str().unwrap().to_owned()
    }

    fn prepare(&self, id: &str, agent: &str) -> Prepared {
        let output = self.command().env("WS_LOOP_ID", "inherited-wrong-loop")
            .args(["agent", "prepare", "--agent", agent, "--mode", "headless", "--session", id])
            .output().unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
