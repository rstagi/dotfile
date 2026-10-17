#[allow(dead_code)]
mod common;

use serde_json::json;
use ws_core::{device::Device, ids::Ulid, log::SessionLog};

use std::{path::PathBuf, process::Command};

struct Sandbox {
    home: tempfile::TempDir,
    socket: PathBuf,
    server: String,
    _daemon: common::Daemon,
}

#[test]
fn empty_picker_matches_zsh() {
    let sandbox = Sandbox::new();
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
}

#[test]
fn stopped_workspaces_match_zsh_and_pins_sort_first() {
    let sandbox = Sandbox::seeded(&[("web/zeta", true), ("api/beta", false), ("api/alpha", false)]);
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    let rows = String::from_utf8(output.stdout).unwrap();
    assert!(rows.lines().next().unwrap().contains("📌"));
    // A directory outside the registry must not enter the Rust picker.
    std::fs::create_dir_all(sandbox.home.path().join("worktrees/rogue/unregistered")).unwrap();
    assert_eq!(sandbox.command().arg("_rows").output().unwrap().stdout, rows.as_bytes());
}

impl Sandbox {
    fn new() -> Self { Self::seeded(&[]) }

    fn seeded(workspaces: &[(&str, bool)]) -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("ws.sock");
        let state = home.path().join("state");
        let device = Device::init(&state, "picker-test").unwrap();
        std::fs::create_dir_all(home.path().join(".ws")).unwrap();
        let mut pins = String::new();
        for (index, (name, pinned)) in workspaces.iter().enumerate() {
            let path = home.path().join("worktrees").join(name);
            std::fs::create_dir_all(&path).unwrap();
            let log = SessionLog::new(&state, Ulid::new(), device.device_id);
            log.append(0, "session.created", json!({"alias": index + 1})).unwrap();
            log.append(0, "workspace.created", json!({"workspace_id": name, "path": path, "pinned": pinned, "windows": []})).unwrap();
            if *pinned { pins.push_str(name); pins.push('\n'); }
        }
        std::fs::write(home.path().join(".ws/pins"), pins).unwrap();
        let server = format!("ws-picker-{}-{}", std::process::id(), home.path().file_name().unwrap().to_string_lossy());
        let mut command = common::ws(home.path(), &socket);
        command.env("WS_TMUX_SOCKET", &server);
        let daemon = common::Daemon::start(command, &socket);
        Self { home, socket, server, _daemon: daemon }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command.env("WS_TMUX_SOCKET", &self.server)
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env_remove("WS_SESSION_ID").env_remove("WS_SESSION").env_remove("TMUX").env_remove("TMUX_PANE");
        command
    }

    fn zsh(&self, args: &[&str]) -> Vec<u8> {
        let output = Command::new("zsh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ws/ws.sh"))
            .args(args)
            .env("HOME", self.home.path())
            .env("WS_HOME", self.home.path().join(".ws"))
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server)
            .env_remove("TMUX").env_remove("TMUX_PANE")
            .output().unwrap();
        assert!(output.status.success(), "zsh {args:?}: {output:?}");
        output.stdout
    }

    fn parity(&self, args: &[&str]) {
        let expected = self.zsh(args);
        let actual = self.command().args(args).output().unwrap();
        assert!(actual.status.success(), "Rust {args:?}: {actual:?}");
        assert_eq!(String::from_utf8(actual.stdout).unwrap(), String::from_utf8(expected).unwrap(), "{args:?}");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new("tmux").args(["-L", &self.server, "kill-server"]).output();
    }
}
