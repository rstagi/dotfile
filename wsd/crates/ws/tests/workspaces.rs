mod common;

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Sandbox {
    home: tempfile::TempDir,
    socket: PathBuf,
    server: String,
    _daemon: common::Daemon,
}

#[test]
fn new_creates_a_worktree_a_registered_workspace_and_a_tagged_tmux_session() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    std::fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feat/login",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let wt = sandbox.home.path().join("worktrees/api/feat-login");
    assert_eq!(git(&wt, &["branch", "--show-current"]), "feat/login");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=1\n"
    );
    let orphan = sandbox.home.path().join("worktrees/api/orphan");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            orphan.to_str().unwrap(),
        ],
    );
    let workspaces = sandbox.json(&["list", "--json"]);
    assert_eq!(workspaces.as_array().unwrap().len(), 1);
    assert_eq!(workspaces[0]["id"], "api/feat-login");
    assert_eq!(workspaces[0]["body"]["path"], wt.to_str().unwrap());
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions[0]["workspace_id"], "api/feat-login");
    let window = sandbox.tmux(&[
        "display-message",
        "-p",
        "-t",
        "api/feat-login:",
        "#{window_id}",
    ]);
    assert_eq!(
        sandbox.tmux(&["show-options", "-wqv", "-t", &window, "@ws_session"]),
        sessions[0]["id"].as_str().unwrap()
    );
    assert_eq!(
        sandbox.tmux(&[
            "display-message",
            "-p",
            "-t",
            &window,
            "#{pane_current_path}"
        ]),
        wt.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-live.txt")
    );
}

#[test]
fn add_registers_an_existing_worktree_and_allocates_another_tab() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let wt = sandbox.home.path().join("worktrees/api/feat-login");
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        &repo,
        &["worktree", "add", "-qb", "feat/login", wt.to_str().unwrap()],
    );
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    let output = sandbox.output(&["add", "api/feat-login", "--detach"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-live.txt")
    );
    let output = sandbox.output(&["add", wt.to_str().unwrap(), "--agent", "codex", "--detach"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.normalized_list("ls"),
        include_str!("golden/workspace-added.txt")
    );
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 2);
    assert_ne!(sessions[0]["id"], sessions[1]["id"]);
    sandbox.tmux(&["kill-session", "-t", "api/feat-login"]);
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-stopped.txt")
    );
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let server = format!(
            "ws-workspace-{}-{}",
            std::process::id(),
            home.path().file_name().unwrap().to_str().unwrap()
        );
        let mut command = common::ws(home.path(), &socket);
        command
            .env("WS_ROOT", home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        let daemon = common::Daemon::start(command, &socket);
        let sandbox = Self {
            home,
            socket,
            server,
            _daemon: daemon,
        };
        sandbox.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keepalive",
            "/bin/sleep",
            "300",
        ]);
        assert!(common::get(&sandbox.socket, "/v1/ping").starts_with("HTTP/1.1 200"));
        sandbox
    }
    fn command(&self) -> Command {
        let mut command = common::ws(self.home.path(), &self.socket);
        command
            .env("WS_ROOT", self.home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        command
    }
    fn output(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let output = self.output(args);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn repo(&self, name: &str) -> PathBuf {
        let repo = self.home.path().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        git(&repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join(".gitignore"), ".env*\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "seed"]);
        repo
    }
    fn tmux(&self, args: &[&str]) -> String {
        let output = Command::new("tmux")
            .args(["-L", &self.server])
            .args(args)
            .env("HOME", self.home.path())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim_end().into()
    }
    fn normalized_list(&self, command: &str) -> String {
        let output = self.output(&[command]);
        assert!(output.status.success(), "{output:?}");
        let root = self.home.path().join("worktrees");
        let mut rows = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| {
                let mut fields: Vec<_> = line.split('\t').map(str::to_owned).collect();
                fields[2] = fields[2]
                    .replace(root.canonicalize().unwrap().to_str().unwrap(), "<ROOT>")
                    .replace(root.to_str().unwrap(), "<ROOT>");
                if fields[4].starts_with('@') {
                    fields[4] = "<WINDOW>".into();
                }
                fields.join("\t")
            })
            .collect::<Vec<_>>();
        rows.sort();
        format!("{}\n", rows.join("\n"))
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.server, "kill-server"])
            .output();
    }
}
fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("HOME", path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}
