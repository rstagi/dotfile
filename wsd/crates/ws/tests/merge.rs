mod common;

use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Sandbox {
    root: tempfile::TempDir,
    socket: PathBuf,
    server: String,
    _daemon: common::Daemon,
}

#[test]
fn declining_merge_preserves_the_pr_main_and_worktree() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    let output = sandbox.merge(&wt, "n\n");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("golden/merge-declined.txt")
    );
    assert_eq!(
        git(&sandbox.repo(), &["log", "-1", "--format=%s", "main"]),
        "initial"
    );
    assert!(wt.is_dir());
    assert!(!sandbox.gh_log().contains("pr merge"));
}

#[test]
fn merging_updates_main_and_keeps_the_worktree_when_deletion_is_declined() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "ship it"]);
    git(&wt, &["push", "-q", "origin", "feat/ship"]);
    sandbox.land_squash();
    let output = sandbox.merge(&wt, "y\nn\n");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.normalized(&output.stdout),
        include_str!("golden/merge-kept.txt")
    );
    assert!(sandbox.gh_log().contains("pr merge feat/ship --squash"));
    assert_eq!(
        git(&sandbox.repo(), &["log", "-1", "--format=%s", "main"]),
        "squashed: ship it"
    );
    assert!(wt.is_dir());
    assert!(!git(&sandbox.repo(), &["branch", "--list", "feat/ship"]).is_empty());
}

#[test]
fn confirmed_deletion_removes_squashed_branches_tabs_pins_and_registry_rows() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "ship it"]);
    git(&wt, &["push", "-q", "origin", "feat/ship"]);
    sandbox.land_squash();
    for args in [
        ["add", "merge-me/feat-ship", "--detach"].as_slice(),
        ["pin", "merge-me/feat-ship"].as_slice(),
    ] {
        let output = sandbox.command().args(args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let output = sandbox.merge(&wt, "y\ny\n");
    assert!(output.status.success(), "{output:?}");
    assert!(!wt.exists());
    assert_eq!(git(&sandbox.repo(), &["branch", "--list", "feat/ship"]), "");
    assert_eq!(
        git(
            &sandbox.repo(),
            &["ls-remote", "--heads", "origin", "feat/ship"]
        ),
        ""
    );
    let output = sandbox.command().args(["list", "--json"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );
    let output = Command::new("tmux")
        .args([
            "-L",
            &sandbox.server,
            "has-session",
            "-t",
            "=merge-me/feat-ship",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn dirty_worktrees_require_a_separate_force_confirmation_after_merging() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&wt, &["push", "-q", "origin", "feat/ship"]);
    std::fs::write(wt.join("stray.txt"), "junk\n").unwrap();
    let output = sandbox.merge(&wt, "y\ny\nn\n");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.normalized(&output.stdout),
        include_str!("golden/merge-dirty-kept.txt")
    );
    assert!(wt.is_dir());
    assert!(
        !git(
            &sandbox.repo(),
            &["ls-remote", "--heads", "origin", "feat/ship"]
        )
        .is_empty()
    );
    let output = sandbox.merge(&wt, "y\ny\ny\n");
    assert!(output.status.success(), "{output:?}");
    assert!(!wt.exists());
    assert_eq!(git(&sandbox.repo(), &["branch", "--list", "feat/ship"]), "");
    assert_eq!(
        git(
            &sandbox.repo(),
            &["ls-remote", "--heads", "origin", "feat/ship"]
        ),
        ""
    );
}

#[test]
fn merge_refuses_directories_outside_a_workspace() {
    let sandbox = Sandbox::new();
    for cwd in [sandbox.repo(), sandbox.root.path().to_owned()] {
        let output = sandbox.merge(&cwd, "");
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            include_str!("golden/merge-outside.txt")
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn merge_uses_the_current_git_workspace_or_the_tmux_workspace_outside_git() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    let nested = wt.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let pane = sandbox.tmux(&[
        "display-message",
        "-p",
        "-t",
        "=merge-me/feat-ship:",
        "#{pane_id}",
    ]);
    for cwd in [&nested, &sandbox.repo(), sandbox.root.path()] {
        let mut command = sandbox.command();
        command.env("TMUX", "sandbox").env("TMUX_PANE", &pane);
        let output = sandbox.input(command, cwd, "merge", "n\n");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            include_str!("golden/merge-declined.txt")
        );
    }
}

#[test]
fn merge_refuses_detached_head_and_a_branch_without_a_pr() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&wt, &["checkout", "-q", "--detach"]);
    let output = sandbox.merge(&wt, "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .replace(wt.canonicalize().unwrap().to_str().unwrap(), "<WT>"),
        include_str!("golden/merge-detached.txt")
    );
    git(&wt, &["checkout", "-q", "feat/ship"]);
    sandbox.fake_gh("#!/bin/sh\nexit 1\n");
    let output = sandbox.merge(&wt, "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/merge-no-pr.txt")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn popup_reports_merge_errors_and_waits_for_enter() {
    use std::{io::Read, sync::mpsc, time::Duration};
    let sandbox = Sandbox::new();
    let mut child = sandbox
        .command()
        .arg("_merge_popup")
        .current_dir(sandbox.root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![0; include_bytes!("golden/merge-popup.txt").len()];
        let result = stdout.read_exact(&mut bytes).map(|()| bytes);
        sender.send(result).unwrap();
    });
    let bytes = receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(bytes, include_bytes!("golden/merge-popup.txt"));
    assert!(child.try_wait().unwrap().is_none());
    child.stdin.take().unwrap().write_all(b"\n").unwrap();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/merge-outside.txt")
    );
    let output = sandbox.input(sandbox.command(), sandbox.root.path(), "_merge_popup", "");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn an_already_merged_pr_can_continue_but_other_merge_failures_stop_before_fetch() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    sandbox.land_squash();
    sandbox.fake_gh(r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_GH_LOG"
case "$2" in
view) if [ "$5" = state ]; then echo "${WS_TEST_PR_STATE:-MERGED}"; else echo '#7 Ship it (https://github.com/acme/merge-me/pull/7)'; fi ;;
merge) exit 1 ;;
esac
"#);
    let output = sandbox.merge(&wt, "y\nn\n");
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("  (already merged)\n")
    );
    assert_eq!(
        git(&sandbox.repo(), &["log", "-1", "--format=%s", "main"]),
        "squashed: ship it"
    );
    let mut command = sandbox.command();
    command.env("WS_TEST_PR_STATE", "OPEN");
    let output = sandbox.input(command, &wt, "merge", "y\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ws: merge: gh pr merge failed\n"
    );
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("updating")
    );
    assert!(wt.is_dir());
}

#[test]
fn popup_uses_the_same_merge_flow_and_waits_after_a_successful_deletion() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&wt, &["push", "-q", "origin", "feat/ship"]);
    let output = sandbox.input(sandbox.command(), &wt, "_merge_popup", "n\n\n");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{}{}",
            include_str!("golden/merge-declined.txt"),
            include_str!("golden/merge-popup.txt")
        )
    );
    let output = sandbox.input(sandbox.command(), &wt, "_merge_popup", "y\ny\n\n");
    assert!(output.status.success(), "{output:?}");
    assert!(!wt.exists());
    assert!(
        output
            .stdout
            .ends_with(include_bytes!("golden/merge-popup.txt"))
    );
}

#[test]
fn confirmation_requires_a_complete_yes_line_and_declines_eof() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    for input in ["", "y", "yes\n", "Y-no\n"] {
        let output = sandbox.merge(&wt, input);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            include_str!("golden/merge-declined.txt")
        );
    }
    assert!(!sandbox.gh_log().contains("pr merge"));
}

#[test]
fn merge_honors_the_method_and_updates_a_default_branch_that_is_not_checked_out() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    git(&sandbox.repo(), &["branch", "-m", "main", "trunk"]);
    git(&sandbox.repo(), &["push", "-q", "origin", "trunk"]);
    git(
        &sandbox.repo(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    git(&sandbox.repo(), &["checkout", "-qb", "local-work"]);
    let original = git(&sandbox.repo(), &["rev-parse", "HEAD"]);
    git(&sandbox.repo(), &["fetch", "-q", "origin", "main:trunk"]);
    sandbox.land_squash();
    git(&sandbox.repo(), &["fetch", "-q", "origin"]);
    git(
        &sandbox.repo(),
        &["push", "-q", "origin", "refs/remotes/origin/main:trunk"],
    );
    let mut command = sandbox.command();
    command.env("WS_MERGE_METHOD", "rebase");
    let output = sandbox.input(command, &wt, "merge", "Y\nn\n");
    assert!(output.status.success(), "{output:?}");
    assert!(sandbox.gh_log().contains("pr merge feat/ship --rebase"));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("→ updating trunk")
    );
    assert_eq!(
        git(&sandbox.repo(), &["log", "-1", "--format=%s", "trunk"]),
        "squashed: ship it"
    );
    assert_eq!(
        git(&sandbox.repo(), &["branch", "--show-current"]),
        "local-work"
    );
    assert_eq!(git(&sandbox.repo(), &["rev-parse", "HEAD"]), original);
}

#[test]
fn failed_fetch_or_fast_forward_preserves_the_worktree_and_branch() {
    for diverged in [false, true] {
        let sandbox = Sandbox::new();
        let wt = sandbox.workspace("feat/ship");
        if diverged {
            sandbox.land_squash();
            git(
                &sandbox.repo(),
                &["commit", "-q", "--allow-empty", "-m", "local change"],
            );
        } else {
            git(
                &sandbox.repo(),
                &[
                    "remote",
                    "set-url",
                    "origin",
                    sandbox.root.path().join("missing.git").to_str().unwrap(),
                ],
            );
        }
        let head = git(&sandbox.repo(), &["rev-parse", "main"]);
        let output = sandbox.merge(&wt, "y\n");
        assert_eq!(output.status.code(), Some(1));
        let message = if diverged {
            "ws: merge: cannot fast-forward main\n"
        } else {
            "ws: merge: fetch failed\n"
        };
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.ends_with(message), "{stderr}");
        assert!(stderr.contains("fatal:"), "{stderr}");
        assert!(wt.is_dir());
        assert_eq!(git(&sandbox.repo(), &["rev-parse", "main"]), head);
        assert!(!git(&sandbox.repo(), &["branch", "--list", "feat/ship"]).is_empty());
    }
}

#[test]
fn merge_reports_no_pr_when_the_github_cli_is_unavailable() {
    let sandbox = Sandbox::new();
    let wt = sandbox.workspace("feat/ship");
    let bin = sandbox.root.path().join("bin");
    std::fs::remove_file(bin.join("gh")).unwrap();
    let real_git = Command::new("/usr/bin/which").arg("git").output().unwrap();
    assert!(real_git.status.success());
    std::os::unix::fs::symlink(
        String::from_utf8(real_git.stdout).unwrap().trim(),
        bin.join("git"),
    )
    .unwrap();
    let mut command = sandbox.command();
    command.env("PATH", &bin);
    let output = sandbox.input(command, &wt, "merge", "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/merge-no-pr.txt")
    );
    assert!(output.stdout.is_empty());
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::create_dir(root.path().join("home")).unwrap();
        std::fs::create_dir(root.path().join("bin")).unwrap();
        let socket = root.path().join("wsd.sock");
        let server = format!(
            "ws-merge-{}-{}",
            std::process::id(),
            root.path().file_name().unwrap().to_str().unwrap()
        );
        let mut command = common::ws(&root.path().join("home"), &socket);
        command
            .env("WS_ROOT", root.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        let daemon = common::Daemon::start(command, &socket);
        let sandbox = Self {
            root,
            socket,
            server,
            _daemon: daemon,
        };
        assert!(common::get(&sandbox.socket, "/v1/ping").starts_with("HTTP/1.1 200"));
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
        std::fs::create_dir(sandbox.repo()).unwrap();
        git(&sandbox.repo(), &["init", "-q", "-b", "main"]);
        git(
            &sandbox.repo(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(&sandbox.repo(), &["config", "user.name", "Test"]);
        git(
            &sandbox.repo(),
            &["commit", "-q", "--allow-empty", "-m", "initial"],
        );
        let origin = sandbox.root.path().join("origin.git");
        git(
            sandbox.root.path(),
            &["init", "-q", "--bare", origin.to_str().unwrap()],
        );
        git(
            &sandbox.repo(),
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&sandbox.repo(), &["push", "-q", "origin", "main"]);
        git(&origin, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(&sandbox.repo(), &["fetch", "-q", "origin"]);
        git(&sandbox.repo(), &["remote", "set-head", "origin", "-a"]);
        sandbox.fake_gh("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FAKE_GH_LOG\"\n[ \"$1 $2\" != 'pr view' ] || echo '#7 Ship it (https://github.com/acme/merge-me/pull/7)'\nexit 0\n");
        sandbox
    }

    fn land_squash(&self) {
        let upstream = self.root.path().join("upstream");
        git(
            self.root.path(),
            &[
                "clone",
                "-q",
                self.root.path().join("origin.git").to_str().unwrap(),
                upstream.to_str().unwrap(),
            ],
        );
        git(
            &upstream,
            &[
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "user.name=Test",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "squashed: ship it",
            ],
        );
        git(&upstream, &["push", "-q", "origin", "main"]);
    }

    fn normalized(&self, bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace(
                self.repo().canonicalize().unwrap().to_str().unwrap(),
                "<REPO>",
            )
            .replace(self.repo().to_str().unwrap(), "<REPO>")
    }

    fn repo(&self) -> PathBuf {
        self.root.path().join("merge-me")
    }

    fn command(&self) -> Command {
        let mut command = common::ws(&self.root.path().join("home"), &self.socket);
        command
            .env("WS_ROOT", self.root.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &self.server)
            .env("FAKE_GH_LOG", self.root.path().join("gh.log"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env_remove("WS_MERGE_METHOD")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        command
    }

    fn workspace(&self, branch: &str) -> PathBuf {
        let output = self
            .command()
            .args([
                "new",
                "--repo",
                self.repo().to_str().unwrap(),
                "--branch",
                branch,
                "--detach",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        self.root
            .path()
            .join("worktrees/merge-me")
            .join(branch.replace('/', "-"))
    }

    fn merge(&self, cwd: &Path, input: &str) -> Output {
        self.input(self.command(), cwd, "merge", input)
    }

    fn input(&self, mut command: Command, cwd: &Path, subcommand: &str, input: &str) -> Output {
        let mut child = command
            .arg(subcommand)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn gh_log(&self) -> String {
        std::fs::read_to_string(self.root.path().join("gh.log")).unwrap()
    }

    fn fake_gh(&self, script: &str) {
        let path = self.root.path().join("bin/gh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn tmux(&self, args: &[&str]) -> String {
        let output = Command::new("tmux")
            .args(["-L", &self.server])
            .args(args)
            .env("HOME", self.root.path().join("home"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim_end().into()
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
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}
