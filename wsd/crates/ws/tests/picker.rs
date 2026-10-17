#[allow(dead_code)]
mod common;

use serde_json::json;
use ws_core::{device::Device, ids::Ulid, log::SessionLog};

use std::{path::PathBuf, process::Command};

struct Server(String);

struct Sandbox {
    _daemon: common::Daemon,
    server: Server,
    socket: PathBuf,
    home: tempfile::TempDir,
}

#[test]
fn pick_preserves_fzf_controls_and_uses_registry_rows() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false), ("api/orphan", true)]);
    let fzf = sandbox.home.path().join("fake-agents/fzf");
    std::fs::write(
        &fzf,
        "#!/bin/sh\ncat > \"$HOME/pick-rows\"\nprintf '%s\\n' \"$@\" > \"$HOME/pick-args\"\nexit 130\n",
    )
    .unwrap();
    std::fs::set_permissions(fzf, std::fs::Permissions::from_mode(0o755)).unwrap();
    let expected = sandbox.command().arg("_rows").output().unwrap().stdout;
    for action in [Some("pick"), None] {
        let mut command = sandbox.command();
        if let Some(action) = action {
            command.arg(action);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            std::fs::read(sandbox.home.path().join("pick-rows")).unwrap(),
            expected
        );
        let args = std::fs::read_to_string(sandbox.home.path().join("pick-args")).unwrap();
        for control in [
            "--ansi",
            "--with-nth\n1",
            "--no-sort",
            "--layout\nreverse",
            "--sync",
            "ctrl-n,ctrl-o,ctrl-r,ctrl-a,ctrl-t,ctrl-v",
            "_summary",
            "_preview {2} {3}",
            "pin {2}",
            "_toggle_sessions",
            "_unread {3}",
            "rm --interactive {2}",
            "_rows",
        ] {
            assert!(args.contains(control), "Missing {control}: {args}");
        }
        assert!(args.contains(env!("CARGO_BIN_EXE_ws")), "{args}");
    }
}

#[test]
fn pick_installs_rust_tmux_bindings_and_preserves_user_hooks() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false)]);
    sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "set-hook",
            "-g",
            "session-window-changed[0]",
            "display-message user-hook",
        ],
    );
    let fzf = sandbox.home.path().join("fake-agents/fzf");
    std::fs::write(&fzf, "#!/bin/sh\ncat >/dev/null\nexit 130\n").unwrap();
    std::fs::set_permissions(fzf, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = sandbox.command().arg("pick").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    for (key, action) in [
        ("w", "pick"),
        ("N", "new"),
        ("M", "_merge_popup"),
        ("a", "add"),
        ("A", "codex"),
        ("G", "grok"),
    ] {
        let binding = sandbox
            .server
            .tmux(sandbox.home.path(), &["list-keys", "-T", "prefix", key]);
        assert!(binding.contains(env!("CARGO_BIN_EXE_ws")), "{binding}");
        assert!(binding.contains(action), "{binding}");
        for value in [
            "WS_HOME=",
            "WS_STATE=",
            "WS_SOCK=",
            "WS_TMUX_SOCKET=",
            sandbox.socket.to_str().unwrap(),
        ] {
            assert!(binding.contains(value), "Missing {value}: {binding}");
        }
        if key == "w" {
            assert!(binding.contains("choose-tree -Zw"), "{binding}");
        }
    }
    let hooks = sandbox
        .server
        .tmux(sandbox.home.path(), &["show-hooks", "-g"]);
    for hook in [
        "session-window-changed[0] display-message user-hook",
        "session-window-changed[77]",
        "client-session-changed[77]",
        "session-window-changed[78]",
        "client-session-changed[78]",
        "_dismiss",
    ] {
        assert!(hooks.contains(hook), "{hooks}");
    }
    assert!(
        sandbox
            .command()
            .arg("pick")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        sandbox
            .server
            .tmux(sandbox.home.path(), &["show-hooks", "-g"]),
        hooks
    );
}

#[test]
fn zsh_picker_checks_run_against_rust_data_commands() {
    let sandbox = Sandbox::new();
    let binary = sandbox.home.path().join("CLI's bin/ws tool");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_ws"), &binary).unwrap();
    let environment = sandbox.command();
    let mut command = Command::new("zsh");
    for (key, value) in environment.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    let output = command
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../ws/tests/picker-rust.test.sh"
        ))
        .env("WS_RUST_BIN", &binary)
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Rust picker shell checks passed.")
    );
}

#[test]
fn empty_picker_matches_zsh() {
    let sandbox = Sandbox::new();
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
}

#[test]
fn stopped_workspaces_match_zsh_and_pins_sort_first() {
    let sandbox = Sandbox::seeded(&[
        ("web/zeta", true),
        ("api/beta", false),
        ("api/alpha", false),
    ]);
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    let rows = String::from_utf8(output.stdout).unwrap();
    assert!(rows.lines().next().unwrap().contains("📌"));
    // A directory outside the registry must not enter the Rust picker.
    std::fs::create_dir_all(sandbox.home.path().join("worktrees/rogue/unregistered")).unwrap();
    assert_eq!(
        sandbox.command().arg("_rows").output().unwrap().stdout,
        rows.as_bytes()
    );
    std::fs::remove_dir_all(sandbox.home.path().join("worktrees/api/alpha")).unwrap();
    assert_eq!(
        sandbox.command().arg("_rows").output().unwrap().stdout,
        rows.as_bytes()
    );
}

#[test]
fn live_picker_matches_zsh_states_unread_and_active_targets() {
    let sandbox = Sandbox::seeded_live(&[
        ("api/alpha", false),
        ("web/zeta", false),
        ("api/orphan", true),
    ]);
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
    let windows = sandbox.server.tmux(
        sandbox.home.path(),
        &["list-windows", "-a", "-F", "#{window_id}"],
    );
    let windows: Vec<_> = windows.lines().collect();
    for window in &windows {
        sandbox.server.tmux(
            sandbox.home.path(),
            &["set", "-wu", "-t", window, "@ws_unread"],
        );
    }
    sandbox.parity(&["_rows"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    let rows = String::from_utf8(output.stdout).unwrap();
    assert!(rows.lines().nth(1).unwrap().ends_with(windows[1]));
    sandbox
        .server
        .tmux(sandbox.home.path(), &["select-window", "-t", windows[0]]);
    sandbox.parity(&["_rows"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap()
            .ends_with(windows[0])
    );
    for window in [windows[0], windows[2]] {
        sandbox.server.tmux(
            sandbox.home.path(),
            &["set", "-w", "-t", window, "@ws_unread", "1"],
        );
    }
    sandbox.parity(&["_rows"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap()
            .ends_with(windows[0])
    );
}

#[test]
fn session_toggle_matches_zsh_expansion_and_collapse() {
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false), ("api/orphan", true)]);
    let collapsed = sandbox.command().arg("_rows").output().unwrap().stdout;
    let output = sandbox.command().arg("_toggle_sessions").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(sandbox.home.path().join(".ws/show-sessions").exists());
    sandbox.parity(&["_rows"]);
    let expanded = sandbox.command().arg("_rows").output().unwrap().stdout;
    assert!(String::from_utf8(expanded).unwrap().contains("grok#3"));
    let output = sandbox.command().arg("_toggle_sessions").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!sandbox.home.path().join(".ws/show-sessions").exists());
    assert_eq!(
        sandbox.command().arg("_rows").output().unwrap().stdout,
        collapsed
    );
    assert!(sandbox.zsh(&["_toggle_sessions"]).is_empty());
    sandbox.parity(&["_rows"]);
}

#[test]
fn unread_toggle_matches_zsh_and_updates_picker_targets() {
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false)]);
    let window = sandbox
        .server
        .tmux(
            sandbox.home.path(),
            &["list-windows", "-a", "-F", "#{window_id}"],
        )
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let output = sandbox
        .command()
        .args(["_unread", &window])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_summary"]);
    let rows = String::from_utf8(sandbox.command().arg("_rows").output().unwrap().stdout).unwrap();
    assert!(rows.lines().next().unwrap().ends_with(&window));
    assert!(sandbox.zsh(&["_unread", &window]).is_empty());
    sandbox.parity(&["_rows"]);
    let output = sandbox
        .command()
        .args(["_unread", &window])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = sandbox
        .command()
        .args(["_unread", &window])
        .output()
        .unwrap();
    assert!(output.status.success());
    sandbox.parity(&["_rows"]);
    sandbox.parity(&["_unread"]);
    sandbox.parity(&["_unread", ""]);
}

#[test]
fn previews_match_zsh_pane_bytes_and_stopped_git_output() {
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false), ("api/orphan", true)]);
    let stopped = sandbox.home.path().join("worktrees/api/orphan");
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "Initial commit",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&stopped)
            .args(args)
            .env("HOME", sandbox.home.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    std::fs::write(stopped.join("untracked"), "preview fixture").unwrap();
    sandbox.parity(&["_preview", stopped.to_str().unwrap(), ""]);
    let window = sandbox
        .server
        .tmux(
            sandbox.home.path(),
            &["list-windows", "-a", "-F", "#{window_id}"],
        )
        .lines()
        .next()
        .unwrap()
        .to_owned();
    sandbox.server.tmux(
        sandbox.home.path(),
        &["send-keys", "-t", &window, "preview-marker", "Enter"],
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !sandbox
        .server
        .tmux(sandbox.home.path(), &["capture-pane", "-p", "-t", &window])
        .contains("preview-marker\npreview-marker")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "The pane did not print the preview marker."
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    sandbox.parity(&[
        "_preview",
        sandbox
            .home
            .path()
            .join("worktrees/api/alpha")
            .to_str()
            .unwrap(),
        &window,
    ]);
    sandbox.parity(&["_preview", "", &window]);
    sandbox.parity(&["_preview"]);
    sandbox.parity(&["_preview", "/missing"]);
}

#[test]
fn picker_preserves_unicode_and_long_names() {
    let sandbox = Sandbox::seeded(&[
        ("répertoire/日本語", false),
        (
            "repository-with-a-long-name/branch-with-a-name-longer-than-the-column",
            true,
        ),
    ]);
    sandbox.parity(&["_rows"]);
}

#[test]
fn repository_discovery_matches_zsh_and_preserves_read_only_mapping() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let home = sandbox.home.path();
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "Initial commit",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let registry = home.join(".loop/repos.json");
    std::fs::create_dir_all(registry.parent().unwrap()).unwrap();
    let mappings = serde_json::to_vec(
        &json!({"a/repo": repo, "b/duplicate": repo, "c/missing": home.join("missing")}),
    )
    .unwrap();
    std::fs::write(&registry, &mappings).unwrap();
    std::fs::set_permissions(&registry, std::fs::Permissions::from_mode(0o444)).unwrap();
    let fzf = home.join("fake-agents/fzf");
    std::fs::write(
        &fzf,
        "#!/bin/sh\ncat > \"$HOME/repo-candidates\"\nprintf '\\n%s\\n' \"$HOME/repo\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fzf, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        home.join("fake-agents").display(),
        std::env::var("PATH").unwrap()
    );
    let zsh = Command::new("zsh").args(["-fc", "source \"$1/ws/lib/workspace.sh\"; source \"$1/ws/lib/picker.sh\"; REGISTRY=\"$HOME/.loop/repos.json\"; WS_REPO_ROOTS=\"\"; pick_repo", "picker"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
        .env("HOME", home).env("PATH", &path).current_dir(home).output().unwrap();
    assert!(zsh.status.success(), "{zsh:?}");
    let expected = std::fs::read(home.join("repo-candidates")).unwrap();
    let output = sandbox
        .command()
        .args(["new", "--branch", "registry-test", "--detach"])
        .env_remove("LOOP_REPO_REGISTRY")
        .env("WS_REPO_ROOTS", "")
        .current_dir(home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(home.join("repo-candidates")).unwrap(),
        expected
    );
    assert_eq!(
        String::from_utf8(expected).unwrap(),
        format!("{}\n", repo.display())
    );
    assert_eq!(std::fs::read(&registry).unwrap(), mappings);
    assert_eq!(
        std::fs::metadata(&registry).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

#[test]
fn registry_membership_excludes_unregistered_tabs_and_previews() {
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false)]);
    let before = sandbox.command().arg("_rows").output().unwrap().stdout;
    let window = sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            "fixture-0",
            "/bin/cat",
        ],
    );
    let path = sandbox.home.path().join("worktrees/api/alpha");
    for (key, value) in [
        ("@ws_path", path.to_str().unwrap()),
        ("@ws_slot", "9"),
        ("@ws_agent", "claude"),
        ("@ws_state", "waiting"),
    ] {
        sandbox.server.tmux(
            sandbox.home.path(),
            &["set", "-w", "-t", &window, key, value],
        );
    }
    assert_eq!(
        sandbox.command().arg("_rows").output().unwrap().stdout,
        before
    );
    let preview = sandbox
        .command()
        .args(["_preview", path.to_str().unwrap(), &window])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    assert!(preview.stdout.is_empty());
    let rogue = sandbox.home.path().join("worktrees/api/review");
    std::fs::create_dir_all(&rogue).unwrap();
    let preview = sandbox
        .command()
        .args(["_preview", rogue.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    assert!(preview.stdout.is_empty());
}

impl Sandbox {
    fn new() -> Self {
        Self::seeded(&[])
    }

    fn seeded(workspaces: &[(&str, bool)]) -> Self {
        Self::fixture(workspaces, false)
    }

    fn seeded_live(workspaces: &[(&str, bool)]) -> Self {
        Self::fixture(workspaces, true)
    }

    fn fixture(workspaces: &[(&str, bool)], live: bool) -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("ws.sock");
        let server = Server(format!(
            "ws-picker-{}-{}",
            std::process::id(),
            home.path().file_name().unwrap().to_string_lossy()
        ));
        let state = home.path().join("state");
        let device = Device::init(&state, "picker-test").unwrap();
        std::fs::create_dir_all(home.path().join(".ws")).unwrap();
        let mut pins = String::new();
        for (index, (name, pinned)) in workspaces.iter().enumerate() {
            let path = home.path().join("worktrees").join(name);
            std::fs::create_dir_all(&path).unwrap();
            let log = SessionLog::new(&state, Ulid::new(), device.device_id);
            log.append(0, "session.created", json!({"alias": index * 10 + 1}))
                .unwrap();
            let mut windows = Vec::new();
            if live && !name.ends_with("orphan") {
                for (slot, (agent, state)) in [
                    ("claude", "idle"),
                    ("codex", "working"),
                    ("grok", "waiting"),
                ]
                .iter()
                .enumerate()
                {
                    let session_id = Ulid::new().to_string();
                    let session_name = format!("fixture-{index}");
                    let window = if slot == 0 {
                        server.tmux(
                            home.path(),
                            &[
                                "-f",
                                "/dev/null",
                                "new-session",
                                "-d",
                                "-P",
                                "-F",
                                "#{window_id}",
                                "-s",
                                &session_name,
                                "/bin/cat",
                            ],
                        )
                    } else {
                        server.tmux(
                            home.path(),
                            &[
                                "new-window",
                                "-d",
                                "-P",
                                "-F",
                                "#{window_id}",
                                "-t",
                                &session_name,
                                "/bin/cat",
                            ],
                        )
                    };
                    for (key, value) in [
                        ("@ws_path", path.to_str().unwrap()),
                        ("@ws_session", &session_id),
                        ("@ws_slot", &(slot + 1).to_string()),
                        ("@ws_agent", agent),
                        ("@ws_state", state),
                    ] {
                        server.tmux(home.path(), &["set", "-w", "-t", &window, key, value]);
                    }
                    if slot == 1 {
                        server.tmux(home.path(), &["select-window", "-t", &window]);
                    }
                    if slot == 2 {
                        server.tmux(
                            home.path(),
                            &["set", "-w", "-t", &window, "@ws_unread", "1"],
                        );
                    }
                    windows.push(json!({"window_id": window, "session_id": session_id, "agent": agent, "slot": slot + 1}));
                }
            }
            log.append(
                0,
                "workspace.created",
                json!({"workspace_id": name, "path": path, "pinned": pinned, "windows": windows}),
            )
            .unwrap();
            if *pinned {
                pins.push_str(name);
                pins.push('\n');
            }
        }
        std::fs::write(home.path().join(".ws/pins"), pins).unwrap();
        let mut command = common::ws(home.path(), &socket);
        command
            .env("WS_TMUX_SOCKET", &server.0)
            .env("WS_ROOT", home.path().join("worktrees"))
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        let daemon = common::Daemon::start(command, &socket);
        Self {
            home,
            socket,
            server,
            _daemon: daemon,
        }
    }

    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env("WS_TMUX_SOCKET", &self.server.0)
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env_remove("WS_SESSION_ID")
            .env_remove("WS_SESSION")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command
    }

    fn zsh(&self, args: &[&str]) -> Vec<u8> {
        let selector = self.home.path().join(".ws/backend");
        std::fs::write(&selector, "zsh\n").unwrap();
        let output = Command::new("zsh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../ws/ws.sh"))
            .args(args)
            .env("HOME", self.home.path())
            .env("WS_HOME", self.home.path().join(".ws"))
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server.0)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        std::fs::write(&selector, "rust\n").unwrap();
        assert!(output.status.success(), "zsh {args:?}: {output:?}");
        output.stdout
    }

    fn parity(&self, args: &[&str]) {
        let expected = self.zsh(args);
        let actual = self.command().args(args).output().unwrap();
        assert!(actual.status.success(), "Rust {args:?}: {actual:?}");
        assert_eq!(
            String::from_utf8(actual.stdout).unwrap(),
            String::from_utf8(expected).unwrap(),
            "{args:?}"
        );
    }
}

impl Server {
    fn tmux(&self, home: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("tmux")
            .args(["-L", &self.0])
            .args(args)
            .env("HOME", home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim_end().into()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.0, "kill-server"])
            .output();
    }
}
