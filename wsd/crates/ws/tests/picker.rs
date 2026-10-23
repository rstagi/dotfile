#[path = "../../ws-daemon/src/import/fixtures/mod.rs"]
mod fixtures;

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
    home: common::SandboxRoot,
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
            "ctrl-n,ctrl-o,ctrl-r,alt-n,alt-o,alt-r,ctrl-a,ctrl-t,ctrl-v",
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
fn installed_tmux_bindings_parse_and_keep_the_workspace_path() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false)]);
    let fzf = sandbox.home.path().join("fake-agents/fzf");
    std::fs::write(&fzf, "#!/bin/sh\ncat >/dev/null\nexit 130\n").unwrap();
    std::fs::set_permissions(fzf, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = sandbox.command().arg("pick").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    // A quote and a space in the directory check the tmux quoting and the sh quoting.
    let directory = sandbox.home.path().join("worktrees/api/it's alpha");
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let directory = directory.to_str().unwrap();
    let window = sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            "fixture-0:",
            "-c",
            directory,
            "/bin/cat",
        ],
    );
    sandbox.server.tmux(
        sandbox.home.path(),
        &["set", "-w", "-t", &window, "@ws_path", directory],
    );
    let parser = Parser {
        sandbox: &sandbox,
        window: &window,
    };
    for (key, action) in [("w", "pick"), ("N", "new"), ("M", "_merge_popup")] {
        let binding = parser.binding(key);
        let (name, condition) = parser.parse(&binding);
        assert_eq!(name, "if-shell", "{binding}");
        assert_eq!(condition[0], "-F", "{binding}");
        let (name, popup) = parser.parse(&condition[2]);
        assert_eq!(name, "display-popup", "{binding}");
        let start = popup.iter().position(|argument| argument == "-d");
        let start = &popup[start.expect("The popup has no -d option.") + 1];
        assert_eq!(parser.expand(start), directory, "{binding}");
        let words = shell_words::split(popup.last().unwrap()).unwrap();
        assert_eq!(words.last().unwrap(), action, "{binding}");
        assert!(words.iter().any(|word| word == env!("CARGO_BIN_EXE_ws")));
        if key == "w" {
            assert_eq!(parser.parse(&condition[3]).0, "choose-tree", "{binding}");
        }
    }
    for (key, agent) in [("a", None), ("A", Some("codex")), ("G", Some("grok"))] {
        let binding = parser.binding(key);
        let (name, condition) = parser.parse(&binding);
        assert_eq!(name, "if-shell", "{binding}");
        let (name, run) = parser.parse(&condition[2]);
        assert_eq!(name, "run-shell", "{binding}");
        let words = shell_words::split(&parser.expand(&run[0])).unwrap();
        let add = words.iter().position(|word| word == "add").unwrap();
        assert_eq!(words[add + 1], directory, "{binding}");
        let options: Vec<_> = agent
            .into_iter()
            .flat_map(|agent| ["--agent", agent])
            .collect();
        assert_eq!(words[add + 2..], options, "{binding}");
    }
    let hooks = sandbox
        .server
        .tmux(sandbox.home.path(), &["show-hooks", "-g"]);
    for hook in ["session-window-changed[78] ", "client-session-changed[78] "] {
        let line = hooks.lines().find(|line| line.starts_with(hook)).unwrap();
        let (name, condition) = parser.parse(&line[hook.len()..]);
        assert_eq!(name, "if-shell", "{line}");
        let (name, run) = parser.parse(&condition[2]);
        assert_eq!(name, "run-shell", "{line}");
        assert_eq!(run[0], "-b", "{line}");
        let words = shell_words::split(&parser.expand(&run[1])).unwrap();
        assert_eq!(
            words[words.len() - 2..],
            ["_dismiss", window.as_str()],
            "{line}"
        );
    }
}

/// Uses the tmux parser to read the commands that the picker installs.
struct Parser<'a> {
    sandbox: &'a Sandbox,
    window: &'a str,
}

impl Parser<'_> {
    /// Returns the command of the `key` binding in the prefix table.
    fn binding(&self, key: &str) -> String {
        let line = self.tmux(&["list-keys", "-T", "prefix", key]);
        // list-keys aligns its columns with spaces.
        line.split_once(" prefix ")
            .and_then(|(_, rest)| rest.trim_start().strip_prefix(key))
            .map(|command| command.trim_start().to_owned())
            .unwrap_or_else(|| panic!("The binding has an unknown format: {line}"))
    }

    /// Parses one tmux command. Returns its name and its arguments after the parser removes the quotes.
    fn parse(&self, command: &str) -> (String, Vec<String>) {
        let home = self.sandbox.home.path();
        let file = home.join("parse.tmux");
        std::fs::write(&file, command).unwrap();
        let output = Command::new("tmux")
            .args(["-S", &self.sandbox.server.0, "source-file", "-n"])
            .arg(&file)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "tmux cannot parse {command}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // new-window gives each parsed argument to the recorder as one argv word.
        let recorder = home.join("record-arguments");
        let record = home.join("record-arguments.out");
        std::fs::write(
            &recorder,
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$0.tmp\" && mv \"$0.tmp\" \"$0.out\"\n",
        )
        .unwrap();
        std::fs::set_permissions(
            &recorder,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let (name, arguments) = command.split_once(' ').unwrap_or((command, ""));
        std::fs::write(
            &file,
            format!(
                "new-window -d -t fixture-0: {} {arguments}\n",
                recorder.display()
            ),
        )
        .unwrap();
        let _ = std::fs::remove_file(&record);
        self.tmux(&["source-file", file.to_str().unwrap()]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !record.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "The recorder did not run."
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let record = std::fs::read_to_string(record).unwrap();
        let arguments = record
            .strip_suffix('\0')
            .map(|record| record.split('\0').map(str::to_owned).collect())
            .unwrap_or_default();
        (name.to_owned(), arguments)
    }

    /// Expands a tmux format for the workspace window.
    fn expand(&self, format: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", self.window, format])
    }

    fn tmux(&self, args: &[&str]) -> String {
        self.sandbox.server.tmux(self.sandbox.home.path(), args)
    }
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
    sandbox.parity("empty/rows", &["_rows"]);
    sandbox.parity("empty/summary", &["_summary"]);
}

#[test]
fn directory_sessions_follow_workspaces_and_group_by_cwd() {
    let sandbox = Sandbox::seeded(&[("api/alpha", false)]);
    let first = sandbox.start_directory("notes with spaces", "claude");
    let other = sandbox.start_directory("other notes", "grok");
    let second = sandbox.start_directory("notes with spaces", "codex");
    let rows = String::from_utf8(sandbox.run(&["_rows"])).unwrap();
    let rows: Vec<_> = rows
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect();
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows.iter().all(|fields| fields.len() == 3), "{rows:?}");
    assert!(rows[0][0].contains("api"));
    assert!(rows[1][0].contains("scratch"));
    assert_eq!(&rows[1][1..], &["", ""]);
    assert_eq!(rows[2][1], first["cwd"].as_str().unwrap());
    assert!(rows[2][0].contains("notes with spaces"));
    assert!(rows[2][0].contains("2 sessions"));
    assert!(
        [
            first["window_id"].as_str().unwrap(),
            second["window_id"].as_str().unwrap()
        ]
        .contains(&rows[2][2])
    );
    assert_eq!(rows[3][1], other["cwd"].as_str().unwrap());
    assert_eq!(rows[3][2], other["window_id"].as_str().unwrap());
    assert!(rows[3][0].contains("grok"));
    let list = String::from_utf8(sandbox.run(&["list"])).unwrap();
    assert_eq!(list.lines().count(), 1);
    assert!(!list.contains("notes"));
    let workspaces: serde_json::Value =
        serde_json::from_slice(&sandbox.run(&["list", "--json"])).unwrap();
    assert_eq!(workspaces.as_array().unwrap().len(), 1);
}

#[test]
fn child_places_nest_under_their_parent_session_and_show_names() {
    let sandbox = Sandbox::new();
    let parent = sandbox.start_place(json!({"name":"orchestrator"}), "a-orch");
    let child = sandbox.start_place(
        json!({"name":"child-notes", "parent_session_id":parent["session_id"]}),
        "b-child",
    );
    let repo = sandbox.home.path().join("api");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "initial",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let response = common::request(
        &sandbox.socket,
        "POST",
        "/v1/workspaces",
        &json!({"repo":repo, "branch":"feature", "agent":"claude", "parent_session_id":parent["session_id"]}).to_string(),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let gone = sandbox.start_place(json!({}), "c-gone");
    let orphan = sandbox.start_place(json!({"parent_session_id":gone["session_id"]}), "d-orphan");
    let done = sandbox
        .command()
        .env("WS_SESSION_ID", gone["session_id"].as_str().unwrap())
        .arg("done")
        .output()
        .unwrap();
    assert!(done.status.success(), "{done:?}");
    let rows = String::from_utf8(sandbox.run(&["_rows"])).unwrap();
    let rows: Vec<_> = rows
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect();
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(rows.iter().all(|fields| fields.len() == 3), "{rows:?}");
    assert!(rows[0][0].contains("scratch"), "{rows:?}");
    assert!(rows[1][0].contains("orchestrator") && !rows[1][0].contains('└'));
    assert_eq!(rows[1][2], parent["window_id"].as_str().unwrap());
    assert!(rows[2][0].contains("└ ") && rows[2][0].contains("child-notes"));
    assert_eq!(rows[2][1], child["cwd"].as_str().unwrap());
    assert!(rows[3][0].contains("└ ") && rows[3][0].contains("feature"));
    assert!(rows[3][1].ends_with("worktrees/api/feature"), "{rows:?}");
    let alias =
        sandbox.json(&["show", orphan["session_id"].as_str().unwrap(), "--json"])["alias"].clone();
    assert!(!rows[4][0].contains('└') && rows[4][0].contains(&format!("#{alias}")));
    assert_eq!(rows[4][1], orphan["cwd"].as_str().unwrap());
}

#[test]
fn directory_sessions_keep_state_icons_counts_and_unread_targets() {
    let sandbox = Sandbox::new();
    let first = sandbox.start_directory("notes", "claude");
    let second = sandbox.start_directory("notes", "codex");
    for (session, state) in [(&first, "working"), (&second, "waiting")] {
        sandbox.server.tmux(
            sandbox.home.path(),
            &[
                "set",
                "-w",
                "-t",
                session["window_id"].as_str().unwrap(),
                "@ws_state",
                state,
            ],
        );
    }
    sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "set",
            "-w",
            "-t",
            second["window_id"].as_str().unwrap(),
            "@ws_unread",
            "1",
        ],
    );
    let rows = String::from_utf8(sandbox.run(&["_rows"])).unwrap();
    let group = rows.lines().nth(1).unwrap();
    assert!(group.contains("\x1b[31m●\x1b[0m"), "{rows}");
    assert!(
        group.ends_with(second["window_id"].as_str().unwrap()),
        "{rows}"
    );
    assert_eq!(
        String::from_utf8(sandbox.run(&["_summary"]))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "\x1b[31m●\x1b[0m 1 waiting · \x1b[33m◐\x1b[0m 1 working · \x1b[36m•\x1b[0m 1 unread"
    );
    sandbox.run(&["_toggle_sessions"]);
    let expanded = String::from_utf8(sandbox.run(&["_rows"])).unwrap();
    assert_eq!(expanded.lines().count(), 4, "{expanded}");
    for session in [&first, &second] {
        assert!(
            expanded
                .lines()
                .any(|line| line.contains("└")
                    && line.ends_with(session["window_id"].as_str().unwrap())),
            "{expanded}"
        );
    }
    assert!(expanded.contains("\x1b[33m◐\x1b[0m"), "{expanded}");
}

#[test]
fn directory_preview_captures_a_registered_non_git_pane() {
    let sandbox = Sandbox::new();
    let session = sandbox.start_directory("notes without git", "claude");
    let window = session["window_id"].as_str().unwrap();
    sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "send-keys",
            "-t",
            window,
            "directory-preview-marker",
            "Enter",
        ],
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let expected = loop {
        let output = sandbox.server.tmux(
            sandbox.home.path(),
            &["capture-pane", "-e", "-p", "-t", window],
        );
        if output.contains("directory-preview-marker\ndirectory-preview-marker") {
            break output;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "The directory pane did not print the marker."
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    for cwd in [session["cwd"].as_str().unwrap(), ""] {
        let output = sandbox
            .command()
            .args(["_preview", cwd, window])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim_end(),
            expected
        );
    }
}

#[test]
fn directory_completion_removes_a_live_session_from_rows_counts_and_preview() {
    let sandbox = Sandbox::new();
    let session = sandbox.start_directory("completed notes", "codex");
    let output = sandbox
        .command()
        .arg("done")
        .env("WS_SESSION_ID", session["session_id"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(sandbox.run(&["_rows"]).is_empty());
    sandbox.parity("empty/summary", &["_summary"]);
    assert!(sandbox.run(&["list"]).is_empty());
    assert!(
        sandbox
            .run(&[
                "_preview",
                session["cwd"].as_str().unwrap(),
                session["window_id"].as_str().unwrap()
            ])
            .is_empty()
    );
}

#[test]
fn directory_picker_excludes_headless_and_terminal_sessions() {
    let sandbox = Sandbox::fixture_with_sessions(
        &[],
        false,
        &[
            ("interactive", "running"),
            ("headless", "running"),
            ("interactive", "finished"),
            ("interactive", "failed"),
        ],
    );
    let rows = String::from_utf8(sandbox.run(&["_rows"])).unwrap();
    assert_eq!(rows.lines().count(), 2, "{rows}");
    let fields: Vec<_> = rows.lines().nth(1).unwrap().split('\t').collect();
    assert_eq!(fields.len(), 3);
    assert_eq!(
        fields[1],
        sandbox.home.path().join("directory-0").to_str().unwrap()
    );
    assert_eq!(fields[2], "");
    assert!(fields[0].contains("\x1b[2m·\x1b[0m"), "{rows}");
    assert_eq!(
        String::from_utf8(sandbox.run(&["_summary"]))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "\x1b[2m·\x1b[0m 1 stopped"
    );
    assert!(sandbox.run(&["list"]).is_empty());
}

#[test]
fn stopped_directory_preview_lists_files_without_git_errors() {
    let sandbox = Sandbox::fixture_with_sessions(&[], false, &[("interactive", "running")]);
    let cwd = sandbox.home.path().join("directory-0");
    for name in ["zeta.txt", "alpha.txt"] {
        std::fs::write(cwd.join(name), "notes").unwrap();
    }
    let output = sandbox
        .command()
        .args(["_preview", cwd.to_str().unwrap(), ""])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "stopped — enter restores its sessions\nalpha.txt\nzeta.txt\n"
    );
}

#[test]
fn stopped_workspaces_match_zsh_and_pins_sort_first() {
    let sandbox = Sandbox::seeded(&[
        ("web/zeta", true),
        ("api/beta", false),
        ("api/alpha", false),
    ]);
    sandbox.parity("stopped/rows", &["_rows"]);
    sandbox.parity("stopped/summary", &["_summary"]);
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
    sandbox.parity("live/initial-rows", &["_rows"]);
    sandbox.parity("live/initial-summary", &["_summary"]);
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
    sandbox.parity("live/read-rows", &["_rows"]);
    let output = sandbox.command().arg("_rows").output().unwrap();
    let rows = String::from_utf8(output.stdout).unwrap();
    assert!(rows.lines().nth(1).unwrap().ends_with(windows[1]));
    sandbox
        .server
        .tmux(sandbox.home.path(), &["select-window", "-t", windows[0]]);
    sandbox.parity("live/focused-rows", &["_rows"]);
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
    sandbox.parity("live/unread-rows", &["_rows"]);
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
fn picker_and_list_keep_legacy_window_activity() {
    let sandbox = Sandbox::seeded_live(&[("api/alpha", false)]);
    let rows = sandbox.command().arg("_rows").output().unwrap();
    let list = sandbox.command().arg("list").output().unwrap();
    let windows = sandbox.server.tmux(
        sandbox.home.path(),
        &["list-windows", "-a", "-F", "#{window_id}"],
    );
    for prefix in ["", "#"] {
        for (index, window) in windows.lines().enumerate() {
            sandbox.server.tmux(
                sandbox.home.path(),
                &[
                    "set",
                    "-w",
                    "-t",
                    window,
                    "@ws_session",
                    &format!("{prefix}{}", index + 2),
                ],
            );
        }
        assert_eq!(
            sandbox.command().arg("_rows").output().unwrap().stdout,
            rows.stdout
        );
        assert_eq!(
            sandbox.command().arg("list").output().unwrap().stdout,
            list.stdout
        );
    }
    sandbox.server.tmux(
        sandbox.home.path(),
        &[
            "set",
            "-w",
            "-t",
            windows.lines().next().unwrap(),
            "@ws_session",
            "999",
        ],
    );
    assert_ne!(
        sandbox.command().arg("list").output().unwrap().stdout,
        list.stdout
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
    sandbox.parity("sessions/expanded", &["_rows"]);
    let expanded = sandbox.command().arg("_rows").output().unwrap().stdout;
    assert!(String::from_utf8(expanded).unwrap().contains("grok#3"));
    let output = sandbox.command().arg("_toggle_sessions").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!sandbox.home.path().join(".ws/show-sessions").exists());
    assert_eq!(
        sandbox.command().arg("_rows").output().unwrap().stdout,
        collapsed
    );
    assert!(sandbox.run(&["_toggle_sessions"]).is_empty());
    sandbox.parity("sessions/reexpanded", &["_rows"]);
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
    sandbox.parity("unread/marked-rows", &["_rows"]);
    sandbox.parity("unread/marked-summary", &["_summary"]);
    let rows = String::from_utf8(sandbox.command().arg("_rows").output().unwrap().stdout).unwrap();
    assert!(rows.lines().next().unwrap().ends_with(&window));
    assert!(sandbox.run(&["_unread", &window]).is_empty());
    sandbox.parity("unread/cleared-rows", &["_rows"]);
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
    sandbox.parity("unread/repeated-rows", &["_rows"]);
    sandbox.parity("unread/no-target", &["_unread"]);
    sandbox.parity("unread/empty-target", &["_unread", ""]);
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
    sandbox.parity(
        "preview/stopped",
        &["_preview", stopped.to_str().unwrap(), ""],
    );
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
    sandbox.parity(
        "preview/workspace-pane",
        &[
            "_preview",
            sandbox
                .home
                .path()
                .join("worktrees/api/alpha")
                .to_str()
                .unwrap(),
            &window,
        ],
    );
    sandbox.parity("preview/pane", &["_preview", "", &window]);
    sandbox.parity("preview/no-target", &["_preview"]);
    sandbox.parity("preview/missing", &["_preview", "/missing"]);
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
    sandbox.parity("unicode/rows", &["_rows"]);
}

#[test]
fn repository_discovery_includes_scratch_and_preserves_read_only_mapping() {
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
    let expected = format!("scratch\n{}\n", repo.display()).into_bytes();
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

    fn start_place(&self, mut body: serde_json::Value, directory: &str) -> serde_json::Value {
        let cwd = self.home.path().join(directory);
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        body["cwd"] = json!(cwd);
        body["agent"] = json!("claude");
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/sessions/start",
            &body.to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let mut result: serde_json::Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        result["cwd"] = json!(cwd);
        result
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        serde_json::from_slice(&self.run(args)).unwrap()
    }

    fn start_directory(&self, name: &str, agent: &str) -> serde_json::Value {
        let cwd = self.home.path().join(name);
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        let response = common::request(
            &self.socket,
            "POST",
            "/v1/sessions/start",
            &json!({"cwd": cwd, "agent": agent}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let mut result: serde_json::Value =
            serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        result["cwd"] = json!(cwd);
        result
    }

    fn seeded(workspaces: &[(&str, bool)]) -> Self {
        Self::fixture(workspaces, false)
    }

    fn seeded_live(workspaces: &[(&str, bool)]) -> Self {
        Self::fixture(workspaces, true)
    }

    fn fixture(workspaces: &[(&str, bool)], live: bool) -> Self {
        Self::fixture_with_sessions(workspaces, live, &[])
    }

    fn fixture_with_sessions(
        workspaces: &[(&str, bool)],
        live: bool,
        sessions: &[(&str, &str)],
    ) -> Self {
        let home = common::SandboxRoot::new();
        fixtures::copy("picker", &home.path().join("picker-fixtures"));
        let socket = home.path().join("ws.sock");
        let server = Server(home.tmux_socket());
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
                    SessionLog::new(
                        &home.path().join("state"),
                        session_id.parse().unwrap(),
                        device.device_id,
                    )
                    .append(0, "session.created", json!({"alias":index * 10 + slot + 2}))
                    .unwrap();
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
        for (index, (kind, lifecycle)) in sessions.iter().enumerate() {
            let cwd = home.path().join(format!("directory-{index}"));
            std::fs::create_dir(&cwd).unwrap();
            let log = SessionLog::new(&state, Ulid::new(), device.device_id);
            log.append(
                0,
                "session.created",
                json!({"alias": index + 100, "cwd": cwd, "kind": kind, "agent": "codex"}),
            )
            .unwrap();
            log.append(0, &format!("session.{lifecycle}"), json!({}))
                .unwrap();
        }
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

    fn run(&self, args: &[&str]) -> Vec<u8> {
        let output = self.command().args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        output.stdout
    }

    fn parity(&self, fixture: &str, args: &[&str]) {
        let path = self.home.path().join("picker-fixtures").join(fixture);
        let mut expected = std::fs::read_to_string(path)
            .unwrap()
            .replace("@HOME@", self.home.path().to_str().unwrap());
        if expected.contains("@COMMIT@") {
            let commit = Command::new("git")
                .arg("-C")
                .arg(args[1])
                .args(["rev-parse", "--short", "HEAD"])
                .env("HOME", self.home.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(commit.status.success(), "{commit:?}");
            expected =
                expected.replace("@COMMIT@", String::from_utf8(commit.stdout).unwrap().trim());
        }
        assert_eq!(
            String::from_utf8(self.run(args)).unwrap(),
            expected,
            "{args:?}"
        );
    }
}

impl Server {
    fn tmux(&self, home: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("tmux")
            .args(["-S", &self.0])
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
