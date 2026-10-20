mod common;

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Sandbox {
    home: common::SandboxRoot,
    socket: PathBuf,
    server: String,
    _daemon: common::Daemon,
}

#[test]
fn open_directory_restores_stopped_sessions_once_with_the_same_identity() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes with spaces");
    std::fs::create_dir(&directory).unwrap();
    assert!(
        sandbox
            .output(&["new", "--cwd", directory.to_str().unwrap(), "--detach"])
            .status
            .success()
    );
    assert!(
        sandbox
            .output(&[
                "add",
                directory.to_str().unwrap(),
                "--agent",
                "codex",
                "--detach"
            ])
            .status
            .success()
    );
    let before = sandbox.json(&["sessions", "--json"]);
    let rows = sandbox.tmux(&["list-windows", "-a", "-F", "#{window_id}\t#{@ws_session}"]);
    for row in rows
        .lines()
        .filter(|row| row.split_once('\t').is_some_and(|(_, id)| !id.is_empty()))
    {
        sandbox.tmux(&["kill-window", "-t", row.split_once('\t').unwrap().0]);
    }
    for _ in 0..2 {
        let output = sandbox
            .command()
            .current_dir(sandbox.home.path())
            .args(["open", "notes with spaces", "--detach"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let after = sandbox.json(&["sessions", "--json"]);
    assert_eq!(
        before
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>(),
        after
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>()
    );
    let rows = sandbox.tmux(&["list-windows", "-a", "-F", "#{@ws_session}"]);
    assert_eq!(rows.lines().filter(|id| !id.is_empty()).count(), 2);
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn interactive_rm_stops_directory_sessions_and_preserves_external_files() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep"), "important notes").unwrap();
    assert!(
        sandbox
            .output(&["new", "--cwd", directory.to_str().unwrap(), "--detach"])
            .status
            .success()
    );
    assert!(
        sandbox
            .output(&[
                "add",
                directory.to_str().unwrap(),
                "--agent",
                "codex",
                "--detach"
            ])
            .status
            .success()
    );
    let other = sandbox.home.path().join("other");
    std::fs::create_dir(&other).unwrap();
    assert!(
        sandbox
            .output(&["new", "--cwd", other.to_str().unwrap(), "--detach"])
            .status
            .success()
    );
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", directory.to_str().unwrap(), "--interactive"],
        "[y/N] ",
        "n",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["state"] == "running")
    );
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", directory.to_str().unwrap(), "--interactive"],
        "[y/N] ",
        "y",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(directory.join("keep")).unwrap(),
        "important notes"
    );
    let directory = directory.canonicalize().unwrap();
    let sessions = sandbox.json(&["sessions", "--json"]);
    for session in sessions.as_array().unwrap() {
        assert_eq!(
            session["state"],
            if session["cwd"] == directory.to_str().unwrap() {
                "finished"
            } else {
                "running"
            }
        );
    }
    let rows = sandbox.tmux(&["list-windows", "-a", "-F", "#{@ws_path}"]);
    assert!(!rows.lines().any(|path| path == directory.to_str().unwrap()));
    assert!(
        rows.lines()
            .any(|path| path == other.canonicalize().unwrap().to_str().unwrap())
    );
}

#[test]
fn rm_scratch_requires_confirmation_even_with_force_and_deletes_only_after_acceptance() {
    let sandbox = Sandbox::new();
    assert!(
        sandbox
            .output(&["new", "--scratch", "--detach"])
            .status
            .success()
    );
    let sessions = sandbox.json(&["sessions", "--json"]);
    let directory = Path::new(sessions[0]["cwd"].as_str().unwrap());
    std::fs::write(directory.join("notes"), "keep until confirmed").unwrap();
    let request = serde_json::json!({"target": directory, "force": true}).to_string();
    let response = common::request(&sandbox.socket, "POST", "/v1/workspaces/remove", &request);
    assert!(!response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("Confirm"), "{response}");
    assert_eq!(sandbox.json(&["sessions", "--json"])[0]["state"], "running");
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", directory.to_str().unwrap(), "--force"],
        "[y/N] ",
        "n",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(directory.join("notes").is_file());
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", directory.to_str().unwrap()],
        "[y/N] ",
        "y",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!directory.exists());
    assert_eq!(
        sandbox.json(&["sessions", "--json"])[0]["state"],
        "finished"
    );
    assert!(directory.parent().unwrap().is_dir());
}

#[test]
fn pin_rejects_directory_places_with_a_clear_error() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox
        .command()
        .current_dir(sandbox.home.path())
        .args(["pin", "notes"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Pins for directory places are not supported.")
    );
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    assert_eq!(sandbox.json(&["sessions", "--json"]), serde_json::json!([]));
}

#[test]
fn new_scratch_starts_a_codex_session_without_a_workspace() {
    let sandbox = Sandbox::new();
    let output = sandbox.output(&["new", "--scratch", "--agent", "codex", "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    let session = &sessions[0];
    assert!(session["workspace_id"].is_null());
    assert_eq!(session["body"]["agent"], "codex");
    let cwd = Path::new(session["cwd"].as_str().unwrap());
    assert_eq!(
        cwd.parent().unwrap(),
        sandbox
            .home
            .path()
            .join(".ws/scratch")
            .canonicalize()
            .unwrap()
    );
    assert!(cwd.is_dir());
    let windows = sandbox.tmux(&["list-windows", "-a", "-F", "#{@ws_session}\t#{@ws_path}"]);
    assert!(
        windows
            .lines()
            .any(|line| line == format!("{}\t{}", session["id"].as_str().unwrap(), cwd.display()))
    );
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn new_cwd_starts_in_a_relative_directory_without_a_branch_prompt() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes with spaces");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox
        .command()
        .args(["new", "--cwd", "notes with spaces", "--detach"])
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(sessions[0]["body"]["agent"], "claude");
    assert!(sessions[0]["workspace_id"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn new_repo_picker_offers_a_scratch_session_without_a_branch_prompt() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let bin = sandbox.home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let fzf = bin.join("fzf");
    std::fs::write(
        &fzf,
        "#!/bin/sh\ncat > \"$WS_TEST_CANDIDATES\"\nprintf 'scratch\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(fzf, std::fs::Permissions::from_mode(0o700)).unwrap();
    let candidates = sandbox.home.path().join("candidates");
    let output = sandbox
        .command()
        .args(["new", "--detach"])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("WS_TEST_CANDIDATES", &candidates)
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        std::fs::read_to_string(candidates)
            .unwrap()
            .lines()
            .any(|line| line == "scratch")
    );
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert!(
        Path::new(sessions[0]["cwd"].as_str().unwrap()).starts_with(
            sandbox
                .home
                .path()
                .join(".ws/scratch")
                .canonicalize()
                .unwrap()
        )
    );
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn new_repo_accepts_a_non_git_directory_without_a_branch_prompt() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox.output(&["new", "--repo", directory.to_str().unwrap(), "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(sessions[0]["workspace_id"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn add_directory_allocates_slot_two_in_the_same_tmux_session() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox.output(&["new", "--cwd", directory.to_str().unwrap(), "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let output = sandbox.output(&[
        "add",
        directory.to_str().unwrap(),
        "--agent",
        "codex",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 2);
    let codex = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["body"]["agent"] == "codex")
        .unwrap();
    assert_eq!(codex["body"]["slot"], 2);
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .all(|session| session["workspace_id"].is_null())
    );
    let windows = sandbox.tmux(&[
        "list-windows",
        "-a",
        "-F",
        "#{session_id}\t#{@ws_path}\t#{@ws_agent}\t#{@ws_slot}",
    ]);
    let directory = directory.canonicalize().unwrap();
    let rows = windows
        .lines()
        .filter(|line| line.split('\t').nth(1) == directory.to_str())
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2, "{windows}");
    assert_eq!(rows[0][0], rows[1][0]);
    assert!(rows.iter().any(|row| row[2] == "codex" && row[3] == "2"));
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn add_without_a_target_uses_the_current_directory_outside_tmux() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox
        .command()
        .args(["add", "--agent", "codex", "--detach"])
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(sessions[0]["body"]["agent"], "codex");
    assert!(sessions[0]["workspace_id"].is_null());
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn add_resolves_a_relative_directory_from_the_callers_directory() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("relative notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox
        .command()
        .args(["add", "relative notes", "--agent", "codex", "--detach"])
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["cwd"],
        directory.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(sessions[0]["workspace_id"].is_null());
}

#[test]
fn add_uses_the_tmux_path_before_the_current_git_workspace() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "caller",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let directory = sandbox.home.path().join("notes");
    std::fs::create_dir(&directory).unwrap();
    let output = sandbox.output(&["new", "--cwd", directory.to_str().unwrap(), "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let windows = sandbox.tmux(&["list-windows", "-a", "-F", "#{pane_id}\t#{@ws_path}"]);
    let directory = directory.canonicalize().unwrap();
    let pane = windows
        .lines()
        .find_map(|line| {
            let (pane, path) = line.split_once('\t')?;
            (path == directory.to_str().unwrap()).then_some(pane)
        })
        .unwrap();
    let output = sandbox
        .command()
        .args(["add", "--agent", "codex", "--detach"])
        .current_dir(sandbox.home.path().join("worktrees/api/caller"))
        .env("TMUX_PANE", pane)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    let codex = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["body"]["agent"] == "codex")
        .unwrap();
    assert_eq!(codex["cwd"], directory.to_str().unwrap());
    assert_eq!(codex["body"]["slot"], 2);
    assert!(codex["workspace_id"].is_null());
    assert_eq!(
        sandbox.json(&["list", "--json"])[0]["body"]["windows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn new_place_flags_reject_conflicting_workspace_flags() {
    let sandbox = Sandbox::new();
    for args in [
        vec!["new", "--cwd", "/missing", "--branch", "feature"],
        vec!["new", "--cwd", "/missing", "--repo", "/repo"],
        vec!["new", "--cwd", "/missing", "--scratch"],
        vec!["new", "--scratch", "--branch", "feature"],
        vec!["new", "--scratch", "--repo", "/repo"],
    ] {
        let output = sandbox.output(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("cannot be used with"), "{stderr}");
        assert!(stderr.contains(args[1]), "{stderr}");
        assert!(
            stderr.contains(if args[1] == "--cwd" { args[3] } else { args[2] }),
            "{stderr}"
        );
    }
    assert_eq!(sandbox.json(&["sessions", "--json"]), serde_json::json!([]));
}

#[test]
fn new_scratch_attaches_by_default() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .focus_command()
        .args(["new", "--scratch"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let focus = std::fs::read_to_string(sandbox.home.path().join("focus")).unwrap();
    assert!(
        focus.lines().any(|line| line == "attach-session"),
        "{focus}"
    );
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["body"]["agent"], "claude");
    assert!(sessions[0]["workspace_id"].is_null());
}

#[test]
fn new_cwd_in_a_git_repository_does_not_create_a_worktree() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&["new", "--cwd", repo.to_str().unwrap(), "--detach"]);
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["cwd"],
        repo.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(sessions[0]["workspace_id"].is_null());
    assert_eq!(git(&repo, &["branch", "--format=%(refname:short)"]), "main");
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn explicit_open_and_remove_accept_worktrees_that_have_no_registry_row() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    for branch in ["opened", "removed", "dirty"] {
        let wt = sandbox.home.path().join("worktrees/api").join(branch);
        std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
        git(
            &repo,
            &["worktree", "add", "-qb", branch, wt.to_str().unwrap()],
        );
    }
    let output = sandbox.output(&["open", "api/opened", "--detach"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
        1
    );
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let dirty = sandbox.home.path().join("worktrees/api/dirty");
    std::fs::write(dirty.join("change"), "dirty\n").unwrap();
    let output = sandbox.output(&["rm", "api/dirty"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("workspace has uncommitted changes:")
    );
    assert_eq!(
        sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
        1
    );
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", "api/removed", "--interactive"],
        "[y/N/b=also delete branch] ",
        "b",
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!sandbox.home.path().join("worktrees/api/removed").exists());
    assert_eq!(git(&repo, &["branch", "--list", "removed"]), "");
    assert_eq!(
        sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
        1
    );
}

#[test]
fn pin_registers_an_explicit_worktree_without_opening_a_tab_or_allocating_a_session() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let wt = sandbox.home.path().join("worktrees/api/orphan");
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        &repo,
        &["worktree", "add", "-qb", "orphan", wt.to_str().unwrap()],
    );
    let output = sandbox.output(&["pin", "api/orphan"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-orphan-pinned.txt")
    );
    assert_eq!(sandbox.json(&["sessions", "--json"]), serde_json::json!([]));
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    std::fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-orphan-pinned.txt")
    );
}

#[test]
fn rm_preserves_unmerged_branches_unless_force_delete_branch_is_explicit() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    for branch in ["keep", "delete"] {
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    branch,
                    "--detach"
                ])
                .status
                .success()
        );
        let wt = sandbox.home.path().join("worktrees/api").join(branch);
        std::fs::write(wt.join("new-file"), "change\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qm", "unmerged"]);
    }
    let output = sandbox.output(&["rm", "api/keep", "--delete-branch"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/workspace-unmerged.txt")
    );
    assert!(!git(&repo, &["branch", "--list", "keep"]).is_empty());
    sandbox.tmux(&["kill-session", "-t", "=api/delete"]);
    let output = sandbox.output(&["rm", "api/delete", "--force-delete-branch"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(git(&repo, &["branch", "--list", "delete"]), "");
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
}

#[test]
fn a_removed_workspace_can_be_created_again_without_its_old_pin() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let create = [
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "again",
        "--detach",
    ];
    assert!(sandbox.output(&create).status.success());
    let owner = sandbox.json(&["list", "--json"])[0]["owner_session_id"].clone();
    assert!(sandbox.output(&["pin", "api/again"]).status.success());
    assert!(sandbox.output(&["rm", "api/again"]).status.success());
    let output = sandbox.output(&create);
    assert!(output.status.success(), "{output:?}");
    let rows = sandbox.json(&["list", "--json"]);
    assert_eq!(rows[0]["owner_session_id"], owner);
    assert_ne!(rows[0]["body"]["pinned"], true);
    assert_eq!(rows[0]["body"]["windows"].as_array().unwrap().len(), 1);
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    std::fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.json(&["list", "--json"]), rows);
}

#[test]
fn bare_ws_creates_its_first_workspace_and_interactive_rm_accepts_cancel_or_branch_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let command = sandbox.focus_command();
    let fzf = sandbox.home.path().join("bin/fzf");
    std::fs::write(
        &fzf,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' {}\n",
            shell_words::quote(repo.to_str().unwrap())
        ),
    )
    .unwrap();
    std::fs::set_permissions(fzf, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = sandbox.tty_output(command, &[], "branch (api): ", "first");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(sandbox.json(&["list", "--json"])[0]["id"], "api/first");
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", "--interactive", "api/first"],
        "[y/N/b=also delete branch] ",
        "n",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
        1
    );
    let output = sandbox.tty_output(
        sandbox.command(),
        &["rm", "--interactive", "api/first"],
        "[y/N/b=also delete branch] ",
        "b",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    assert_eq!(git(&repo, &["branch", "--list", "first"]), "");
}

#[test]
fn new_installs_picker_bindings_without_a_prior_picker() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "keys",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let binding = sandbox.tmux(&["list-keys", "-T", "prefix", "w"]);
    assert!(binding.contains(env!("CARGO_BIN_EXE_ws")), "{binding}");
    assert!(binding.contains("pick"), "{binding}");
    assert!(binding.contains("choose-tree -Zw"), "{binding}");
}

#[test]
fn bare_ws_picks_registered_workspaces_with_pins_first_and_accepts_cancellation() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    for branch in ["alpha", "zeta"] {
        assert!(
            sandbox
                .output(&[
                    "new",
                    "--repo",
                    repo.to_str().unwrap(),
                    "--branch",
                    branch,
                    "--detach"
                ])
                .status
                .success()
        );
    }
    assert!(sandbox.output(&["pin", "api/zeta"]).status.success());
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "orphan",
            sandbox
                .home
                .path()
                .join("worktrees/api/orphan")
                .to_str()
                .unwrap(),
        ],
    );
    let mut command = sandbox.focus_command();
    let fzf = sandbox.home.path().join("bin/fzf");
    std::fs::write(&fzf, "#!/bin/sh\ncat > \"$WS_TEST_ROWS\"\n[ -z \"$WS_TEST_CANCEL\" ] || exit 130\nprintf '\\n'\nhead -1 \"$WS_TEST_ROWS\"\n").unwrap();
    std::fs::set_permissions(&fzf, std::fs::Permissions::from_mode(0o700)).unwrap();
    let rows = sandbox.home.path().join("rows");
    let output = command.env("WS_TEST_ROWS", &rows).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let rows = std::fs::read_to_string(rows).unwrap();
    assert_eq!(rows.lines().count(), 2);
    assert!(
        rows.lines()
            .next()
            .unwrap()
            .split('\t')
            .nth(1)
            .unwrap()
            .ends_with("api/zeta"),
        "{rows}"
    );
    assert!(!rows.contains("orphan"));
    let root = sandbox
        .home
        .path()
        .join("worktrees")
        .canonicalize()
        .unwrap();
    let order = rows
        .lines()
        .map(|row| {
            row.split('\t')
                .nth(1)
                .unwrap()
                .replace(root.to_str().unwrap(), "<ROOT>")
                .replace(
                    sandbox.home.path().join("worktrees").to_str().unwrap(),
                    "<ROOT>",
                )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(order, include_str!("golden/workspace-pick-order.txt"));
    let expected = sandbox.tmux(&["display-message", "-p", "-t", "=api/zeta:", "#{session_id}"]);
    let focus = std::fs::read_to_string(sandbox.home.path().join("focus")).unwrap();
    assert!(
        focus.ends_with(&format!("attach-session\n-t\n{expected}\n")),
        "Expected {expected}: {focus}"
    );
    std::fs::remove_file(sandbox.home.path().join("focus")).unwrap();
    let output = sandbox
        .focus_command()
        .env("WS_TEST_ROWS", sandbox.home.path().join("rows"))
        .env("WS_TEST_CANCEL", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(!sandbox.home.path().join("focus").exists());
}

#[test]
fn open_selects_the_target_tab_clears_unread_and_attaches_or_switches_its_session() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feat/login",
                "--detach"
            ])
            .status
            .success()
    );
    assert!(
        sandbox
            .output(&["add", "api/feat-login", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    let workspaces = sandbox.json(&["list", "--json"]);
    let windows = workspaces[0]["body"]["windows"].as_array().unwrap();
    let first = windows[0]["window_id"].as_str().unwrap();
    let second = windows[1]["window_id"].as_str().unwrap();
    let session = sandbox.tmux(&["display-message", "-p", "-t", first, "#{session_id}"]);
    sandbox.tmux(&["set-option", "-w", "-t", second, "@ws_unread", "1"]);
    let output = sandbox
        .focus_command()
        .args(["open", "#2"])
        .env("TMUX", "")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.tmux(&["display-message", "-p", "-t", second, "#{window_active}"]),
        "1"
    );
    assert_eq!(
        sandbox.tmux(&["show-options", "-wqv", "-t", second, "@ws_unread"]),
        ""
    );
    let focus = std::fs::read_to_string(sandbox.home.path().join("focus")).unwrap();
    assert!(
        focus.ends_with(&format!("attach-session\n-t\n{session}\n")),
        "{focus}"
    );
    let output = sandbox
        .focus_command()
        .args(["open", "api/feat-login"])
        .env("TMUX", "fake")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.tmux(&["display-message", "-p", "-t", second, "#{window_active}"]),
        "1"
    );
    let focus = std::fs::read_to_string(sandbox.home.path().join("focus")).unwrap();
    assert!(
        focus.ends_with(&format!("switch-client\n-t\n{session}\n")),
        "{focus}"
    );
}

#[test]
fn open_restores_stopped_tabs_once_and_preserves_each_session_identity() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feat/login",
                "--detach"
            ])
            .status
            .success()
    );
    assert!(
        sandbox
            .output(&["add", "api/feat-login", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    let before = sandbox.json(&["sessions", "--json"]);
    sandbox.tmux(&["kill-session", "-t", "=api/feat-login"]);
    let children = (0..8)
        .map(|_| {
            sandbox
                .command()
                .args(["open", "api/feat-login", "--detach"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty());
    }
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-reopened.txt")
    );
    let after = sandbox.json(&["sessions", "--json"]);
    assert_eq!(
        before
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>(),
        after
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>()
    );
    let id = before[0]["id"].as_str().unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{id}/stop"),
        "{}",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let workspaces = sandbox.json(&["list", "--json"]);
    let window = workspaces[0]["body"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["session_id"] == id)
        .unwrap()["window_id"]
        .as_str()
        .unwrap();
    assert!(
        !sandbox
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .lines()
            .any(|id| id == window)
    );
    let output = sandbox.output(&["open", id, "--detach"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.json(&["sessions", "--json", id])[0]["state"],
        "running"
    );
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn open_reuses_live_sessions_by_workspace_path_alias_name_and_ulid() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feat/login",
                "--detach"
            ])
            .status
            .success()
    );
    let sessions = sandbox.json(&["sessions", "--json"]);
    let id = sessions[0]["id"].as_str().unwrap();
    let response = common::request(
        &sandbox.socket,
        "POST",
        &format!("/v1/sessions/{id}/name"),
        r#"{"name":"reviewer"}"#,
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let path = sandbox.home.path().join("worktrees/api/feat-login");
    for target in [
        "api/feat-login",
        path.to_str().unwrap(),
        "#1",
        "reviewer",
        id,
    ] {
        let output = sandbox.output(&["open", target, "--detach"]);
        assert!(output.status.success(), "{target}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-live.txt")
    );
    let output = sandbox.output(&["open", "missing", "--detach"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/workspace-missing.txt")
    );
}

#[test]
fn rm_refuses_dirty_worktrees_then_removes_all_tabs_the_worktree_and_the_registry_row() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feat/login",
                "--detach"
            ])
            .status
            .success()
    );
    assert!(
        sandbox
            .output(&["add", "api/feat-login", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    assert!(sandbox.output(&["pin", "api/feat-login"]).status.success());
    let wt = sandbox.home.path().join("worktrees/api/feat-login");
    std::fs::write(wt.join("untracked"), "dirty").unwrap();
    let output = sandbox.output(&["rm", "api/feat-login"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().replace(
            sandbox
                .home
                .path()
                .join("worktrees")
                .canonicalize()
                .unwrap()
                .to_str()
                .unwrap(),
            "<ROOT>"
        ),
        include_str!("golden/workspace-dirty.txt")
    );
    assert!(wt.is_dir());
    assert_eq!(
        sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
        1
    );
    sandbox.tmux(&["has-session", "-t", "=api/feat-login"]);
    let output = sandbox.output(&["rm", wt.to_str().unwrap(), "--force", "--delete-branch"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!wt.exists());
    assert_eq!(git(&repo, &["branch", "--list", "feat/login"]), "");
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    assert!(
        !sandbox
            .tmux(&["list-sessions", "-F", "#{session_name}"])
            .contains("api/feat-login")
    );
    assert!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["state"] == "finished")
    );
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    std::fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    let output = sandbox.output(&["rm", "missing"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/workspace-missing.txt")
    );
}

#[test]
fn pin_toggles_persists_and_survives_a_registry_rebuild() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feat/login",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let output = sandbox.output(&["pin", "api/feat-login"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-pinned.txt")
    );
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    std::fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.json(&["list", "--json"])[0]["body"]["pinned"], true);
    let path = sandbox.home.path().join("worktrees/api/feat-login");
    assert!(
        sandbox
            .output(&["pin", path.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(
        sandbox.json(&["list", "--json"])[0]["body"]["pinned"],
        false
    );
    let output = sandbox.output(&["pin", "missing"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        include_str!("golden/workspace-missing.txt")
    );
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

#[test]
fn new_honors_conductor_includes_and_runs_setup_in_the_worktree() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("shop");
    std::fs::create_dir_all(repo.join(".conductor")).unwrap();
    std::fs::create_dir_all(repo.join("apps/ui")).unwrap();
    std::fs::write(repo.join(".gitignore"), ".env*\n.flags.local.json\n").unwrap();
    std::fs::write(repo.join("apps/ui/.env.local"), "X=1\n").unwrap();
    std::fs::write(repo.join(".flags.local.json"), "{}\n").unwrap();
    std::fs::write(
        repo.join(".conductor/settings.toml"),
        r#"
file_include_globs = """
.env*
.flags.local.json
"""
[scripts]
setup = "touch SETUP-RAN"
"#,
    )
    .unwrap();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "configured",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let wt = sandbox.home.path().join("worktrees/shop/configured");
    assert_eq!(
        std::fs::read_to_string(wt.join(".flags.local.json")).unwrap(),
        "{}\n"
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("apps/ui/.env.local")).unwrap(),
        "X=1\n"
    );
    wait_until(|| wt.join("SETUP-RAN").is_file());
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "configured",
        "--detach",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("workspace exists:")
    );
}

#[test]
fn new_reads_repo_candidates_without_writing_the_loop_mapping() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("mapped");
    let mapping = sandbox.home.path().join(".loop/repos.json");
    std::fs::create_dir_all(mapping.parent().unwrap()).unwrap();
    let contents = serde_json::to_vec(&serde_json::json!({"org/mapped": repo})).unwrap();
    std::fs::write(&mapping, &contents).unwrap();
    std::fs::set_permissions(&mapping, std::fs::Permissions::from_mode(0o400)).unwrap();
    let bin = sandbox.home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let fzf = bin.join("fzf");
    std::fs::write(
        &fzf,
        "#!/bin/sh\ncat > \"$WS_TEST_CANDIDATES\"\nprintf '%s\\n' \"$WS_TEST_PICK\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fzf, std::fs::Permissions::from_mode(0o700)).unwrap();
    let candidates = sandbox.home.path().join("candidates");
    let output = sandbox
        .command()
        .args(["new", "--branch", "chosen", "--detach"])
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("WS_REPO_ROOTS", sandbox.home.path().join("missing"))
        .env("WS_TEST_CANDIDATES", &candidates)
        .env("WS_TEST_PICK", &repo)
        .current_dir(sandbox.home.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        std::fs::read_to_string(candidates)
            .unwrap()
            .lines()
            .any(|line| line == repo.to_str().unwrap())
    );
    assert_eq!(std::fs::read(&mapping).unwrap(), contents);
    assert_eq!(
        std::fs::metadata(&mapping).unwrap().permissions().mode() & 0o777,
        0o400
    );
    assert_eq!(
        git(
            &sandbox.home.path().join("worktrees/mapped/chosen"),
            &["branch", "--show-current"]
        ),
        "chosen"
    );
}

#[test]
fn workspace_creation_preserves_the_calling_session_identity() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("parented");
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/sessions",
        r#"{"name":"orchestrator"}"#,
    );
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    let parent: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let output = sandbox
        .command()
        .args([
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "child",
            "--detach",
        ])
        .env("WS_SESSION_ID", parent["id"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let sessions = sandbox.json(&["sessions", "--json"]);
    let child = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["workspace_id"] == "parented/child")
        .unwrap();
    assert_eq!(child["body"]["parent_session_id"], parent["id"]);
}

#[test]
fn add_uses_the_current_tmux_workspace_when_the_directory_is_outside_git() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "current",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let pane = sandbox.tmux(&["display-message", "-p", "-t", "api/current:", "#{pane_id}"]);
    let output = sandbox
        .command()
        .args(["add", "--agent", "codex", "--detach"])
        .current_dir(sandbox.home.path())
        .env("TMUX_PANE", pane)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox
            .json(&["sessions", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn registered_workspaces_survive_a_registry_rebuild_without_discovering_directories() {
    let mut sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "feat/login",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let before = sandbox.json(&["list", "--json"]);
    let orphan = sandbox.home.path().join("worktrees/api/unregistered");
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
    sandbox._daemon.child.kill().unwrap();
    sandbox._daemon.child.wait().unwrap();
    std::fs::remove_file(sandbox.home.path().join("state/registry.sqlite")).unwrap();
    sandbox._daemon = common::Daemon::start(sandbox.command(), &sandbox.socket);
    assert_eq!(sandbox.json(&["list", "--json"]), before);
    assert_eq!(
        sandbox.normalized_list("list"),
        include_str!("golden/workspace-live.txt")
    );
}

#[test]
fn new_keeps_an_existing_branch_and_uses_the_default_setup_script() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("space repo");
    git(&repo, &["branch", "existing"]);
    let script = repo.join(".ws-setup");
    std::fs::write(&script, "#!/bin/sh\ntouch DEFAULT-SETUP\n").unwrap();
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "existing",
        "--detach",
    ]);
    assert!(output.status.success(), "{output:?}");
    let wt = sandbox.home.path().join("worktrees/space repo/existing");
    wait_until(|| wt.join("DEFAULT-SETUP").is_file());
    assert_eq!(
        git(&wt, &["rev-parse", "HEAD"]),
        git(&repo, &["rev-parse", "existing"])
    );
}

#[test]
fn a_tmux_name_collision_preserves_the_session_and_removes_the_unregistered_worktree() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo("api");
    sandbox.tmux(&[
        "new-session",
        "-d",
        "-s",
        "api/collision",
        "/bin/sleep",
        "300",
    ]);
    let output = sandbox.output(&[
        "new",
        "--repo",
        repo.to_str().unwrap(),
        "--branch",
        "collision",
        "--detach",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("belongs to another workspace")
    );
    assert!(!sandbox.home.path().join("worktrees/api/collision").exists());
    assert_eq!(sandbox.json(&["list", "--json"]), serde_json::json!([]));
    sandbox.tmux(&["has-session", "-t", "=api/collision"]);
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "The workspace operation timed out."
        );
        std::thread::yield_now();
    }
}

impl Sandbox {
    fn tty_output(
        &self,
        environment: Command,
        args: &[&str],
        prompt: &str,
        answer: &str,
    ) -> Output {
        let mut command = Command::new("python3");
        command
            .args([
                "-I",
                "-c",
                include_str!("workspace-tty.py"),
                env!("CARGO_BIN_EXE_ws"),
                prompt,
                answer,
            ])
            .args(args);
        for (key, value) in environment.get_envs() {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command.current_dir(self.home.path()).output().unwrap()
    }

    fn focus_command(&self) -> Command {
        use std::os::unix::fs::PermissionsExt;
        let bin = self.home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let real_tmux = Command::new("zsh")
            .args(["-fc", "command -v tmux"])
            .output()
            .unwrap();
        assert!(real_tmux.status.success());
        let real_tmux = String::from_utf8(real_tmux.stdout).unwrap();
        let script = bin.join("tmux");
        // Global options such as -u and -L come before the command name.
        std::fs::write(&script, format!("#!/bin/sh\ncase \" $* \" in *' attach-session '*|*' switch-client '*) printf '%s\\n' \"$@\" > \"$WS_TEST_FOCUS\"; exit 0 ;; esac\nexec {} \"$@\"\n", shell_words::quote(real_tmux.trim()))).unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = self.command();
        command
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("WS_TEST_FOCUS", self.home.path().join("focus"));
        command
    }

    fn new() -> Self {
        let home = common::SandboxRoot::new();
        let socket = home.path().join("wsd.sock");
        let server = home.tmux_socket();
        let mut command = common::ws(home.path(), &socket);
        command
            .env("WS_ROOT", home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env("WS_AGENT_SHELL", "zsh -fc")
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
            .env("WS_AGENT_SHELL", "zsh -fc")
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
            .args(["-S", &self.server])
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
