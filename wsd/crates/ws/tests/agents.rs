mod common;

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

struct Sandbox {
    home: tempfile::TempDir,
    socket: PathBuf,
    server: String,
    listener: UnixListener,
    _daemon: common::Daemon,
}

#[test]
fn workspace_agents_receive_hooks_identity_slots_and_resume_ids() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    let mut agents = vec![];
    for (i, agent) in ["claude", "codex", "grok"].into_iter().enumerate() {
        let output = if i == 0 {
            sandbox.output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--agent",
                agent,
                "--detach",
            ])
        } else {
            sandbox.output(&["add", "api/feature", "--agent", agent, "--detach"])
        };
        assert!(output.status.success(), "{output:?}");
        let (capture, _running) = sandbox.capture();
        assert_eq!(capture["agent"], agent);
        assert_eq!(capture["id"], capture["legacy_id"]);
        assert_eq!(
            capture["cwd"],
            sandbox.worktree().canonicalize().unwrap().to_str().unwrap()
        );
        let sessions = sandbox.json(&["sessions", "--json"]);
        let session = sessions
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == capture["id"])
            .unwrap();
        let slot_file = sandbox.slots();
        let rows = fs::read_to_string(&slot_file).unwrap();
        assert!(rows.contains(&format!(
            "{}\t{agent}\t\t{}\n",
            i + 1,
            session["id"].as_str().unwrap()
        )));
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        if agent == "claude" {
            let settings =
                Path::new(&args[args.iter().position(|v| v == "--settings").unwrap() + 1]);
            let hooks: Value = serde_json::from_slice(&fs::read(settings).unwrap()).unwrap();
            let hook = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap();
            assert_eq!(
                shell_words::split(hook).unwrap(),
                [env!("CARGO_BIN_EXE_ws"), "hook", "stop"]
            );
            assert!(settings.starts_with(sandbox.home.path()));
        } else if agent == "codex" {
            assert!(args.iter().any(|s| s == "--dangerously-bypass-hook-trust"));
            assert!(args.iter().any(|s| s.starts_with("hooks.Stop=")));
        } else {
            assert!(sandbox.home.path().join(".grok/hooks/ws.json").is_file());
        }
        let id = session["id"].as_str().unwrap();
        let rows = rows.replace(
            &format!("\t{agent}\t\t{id}"),
            &format!("\t{agent}\tresume-{agent}\t{id}"),
        );
        fs::write(slot_file, rows).unwrap();
        let window = sandbox.window(id);
        sandbox.tmux(&["kill-window", "-t", &window]);
        let output = sandbox.output(&["open", id, "--detach"]);
        assert!(output.status.success(), "{output:?}");
        let (resumed, running) = sandbox.capture();
        agents.push(running);
        assert_eq!(resumed["id"], id);
        let args = resumed["argv"].as_array().unwrap();
        assert!(args.windows(2).any(|p| p
            == [
                json!(if agent == "codex" {
                    "resume"
                } else {
                    "--resume"
                }),
                json!(format!("resume-{agent}"))
            ]));
    }
}

#[test]
fn named_agent_commands_run_in_the_current_tab_and_preserve_failed_slots() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (_initial, _running) = sandbox.capture();
    for (agent, exit_code) in [("claude", 0), ("codex", 7), ("grok", 0)] {
        let window = sandbox.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            "api/feature:",
            "-c",
            sandbox.worktree().to_str().unwrap(),
            "/bin/cat",
        ]);
        let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
        let mut command = sandbox.command();
        command
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent, "Review this", "--model", "test-model"]);
        let child = command.spawn().unwrap();
        let (capture, mut connection) = sandbox.capture();
        assert_eq!(capture["workspace"], "api/feature");
        let args: Vec<String> = serde_json::from_value(capture["argv"].clone()).unwrap();
        assert_eq!(
            &args[args.len() - 3..],
            ["Review this", "--model", "test-model"]
        );
        let id = capture["id"].as_str().unwrap();
        assert_eq!(sandbox.window(id), window);
        let output = sandbox
            .command()
            .current_dir(sandbox.worktree())
            .env("TMUX_PANE", &pane)
            .args([agent])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("this tab already runs a ws session")
        );
        connection
            .write_all(exit_code.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(exit_code), "{output:?}");
        assert_eq!(
            sandbox.tmux(&["show-options", "-wqv", "-t", &window, "@ws_slot"]),
            ""
        );
        let rows = fs::read_to_string(sandbox.slots()).unwrap();
        assert_eq!(rows.contains(id), exit_code != 0);
        let session = sandbox.json(&["show", id, "--json"]);
        assert_eq!(
            session["state"],
            if exit_code == 0 {
                "finished"
            } else {
                "running"
            }
        );
        sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    }
}

#[test]
fn a_clean_exit_releases_its_slot_and_reopening_does_not_steal_another_slot() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (first, mut connection) = sandbox.capture();
    let id = first["id"].as_str().unwrap();
    connection.write_all(b"0").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if sandbox.json(&["show", id, "--json"])["state"] == "finished" {
            break;
        }
        assert!(
            Instant::now() < end,
            "The agent exit did not reach the daemon."
        );
    }
    assert!(
        sandbox
            .output(&["add", "api/feature", "--agent", "codex", "--detach"])
            .status
            .success()
    );
    let (second, _running) = sandbox.capture();
    let second_id = second["id"].as_str().unwrap();
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(
        rows.starts_with(&format!("1\tcodex\t\t{second_id}\n")),
        "{rows}"
    );
    assert!(sandbox.output(&["open", id, "--detach"]).status.success());
    let (reopened, _running) = sandbox.capture();
    assert_eq!(reopened["id"], id);
    let rows = fs::read_to_string(sandbox.slots()).unwrap();
    assert!(rows.contains(&format!("1\tcodex\t\t{second_id}\n")));
    assert!(rows.contains(&format!("2\tclaude\t\t{id}\n")));
}

#[test]
fn a_manual_tab_can_run_an_agent_after_it_moves_to_another_tmux_session() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    fs::create_dir_all(sandbox.worktree().parent().unwrap()).unwrap();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "feature",
            sandbox.worktree().to_str().unwrap(),
        ],
    );
    assert!(sandbox.output(&["pin", "api/feature"]).status.success());
    let window = sandbox.tmux(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{window_id}",
        "-s",
        "custom",
        "-c",
        sandbox.worktree().to_str().unwrap(),
        "/bin/cat",
    ]);
    sandbox.tmux(&[
        "set-option",
        "-w",
        "-t",
        &window,
        "@ws_path",
        sandbox.worktree().to_str().unwrap(),
    ]);
    let pane = sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
    let child = sandbox
        .command()
        .env("TMUX_PANE", &pane)
        .current_dir(sandbox.worktree())
        .arg("claude")
        .spawn()
        .unwrap();
    let (capture, mut connection) = sandbox.capture();
    assert_eq!(sandbox.window(capture["id"].as_str().unwrap()), window);
    connection.write_all(b"0").unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    sandbox.tmux(&["display-message", "-p", "-t", &window, "#{pane_id}"]);
}

#[test]
fn split_panes_cannot_prepare_or_finish_the_windows_agent() {
    let sandbox = Sandbox::new();
    let repo = sandbox.repo();
    assert!(
        sandbox
            .output(&[
                "new",
                "--repo",
                repo.to_str().unwrap(),
                "--branch",
                "feature",
                "--detach"
            ])
            .status
            .success()
    );
    let (capture, _running) = sandbox.capture();
    let id = capture["id"].as_str().unwrap();
    let window = sandbox.window(id);
    let pane = sandbox.tmux(&[
        "split-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        &window,
        "/bin/cat",
    ]);
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/launch",
        &json!({"id":id, "pane":pane}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 400"), "{result}");
    let result = common::request(
        &sandbox.socket,
        "POST",
        "/v1/agents/exit",
        &json!({"id":id, "pane":pane, "code":0, "user_tab":true}).to_string(),
    );
    assert!(result.starts_with("HTTP/1.1 200"), "{result}");
    assert_eq!(sandbox.json(&["show", id, "--json"])["state"], "running");
    assert!(fs::read_to_string(sandbox.slots()).unwrap().contains(id));
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let socket = home.path().join("wsd.sock");
        let server = format!(
            "ws-agents-{}-{}",
            std::process::id(),
            home.path().file_name().unwrap().to_str().unwrap()
        );
        let mut command = common::ws(home.path(), &socket);
        let listener = UnixListener::bind(home.path().join("agent.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let bin = home.path().join("fake-agents");
        fs::create_dir_all(&bin).unwrap();
        for agent in ["claude", "codex", "grok"] {
            let script = format!(
                r#"#!/usr/bin/python3
import json, os, socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect({:?})
s.sendall((json.dumps(dict(agent={agent:?}, argv=sys.argv[1:], id=os.getenv('WS_SESSION_ID'), legacy_id=os.getenv('WS_SESSION'), cwd=os.getcwd(), workspace=os.getenv('WS_WORKSPACE')))+'\n').encode())
sys.exit(int(s.recv(1) or b'0'))
"#,
                home.path().join("agent.sock").to_str().unwrap()
            );
            fs::write(bin.join(agent), script).unwrap();
            fs::set_permissions(bin.join(agent), fs::Permissions::from_mode(0o755)).unwrap();
        }
        command
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("WS_ROOT", home.path().join("worktrees"))
            .env("WS_TMUX_SOCKET", &server)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("WS_SESSION")
            .env_remove("WS_SESSION_ID");
        let daemon = common::Daemon::start(command, &socket);
        assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
        let sandbox = Self {
            home,
            socket,
            server,
            listener,
            _daemon: daemon,
        };
        sandbox.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keepalive",
            "/bin/cat",
        ]);
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
        let out = self.output(args);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.server])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim_end().into()
    }
    fn capture(&self) -> (Value, UnixStream) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = vec![];
                    loop {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        if byte[0] == b'\n' {
                            break;
                        }
                        bytes.push(byte[0]);
                    }
                    return (serde_json::from_slice(&bytes).unwrap(), stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "The fake agent did not start.");
                    std::thread::yield_now();
                }
                Err(e) => panic!("{e}"),
            }
        }
    }
    fn repo(&self) -> PathBuf {
        let repo = self.home.path().join("api");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
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
        );
        repo
    }
    fn worktree(&self) -> PathBuf {
        self.home.path().join("worktrees/api/feature")
    }
    fn slots(&self) -> PathBuf {
        PathBuf::from(git(&self.worktree(), &["rev-parse", "--absolute-git-dir"]))
            .join("ws-sessions")
    }
    fn window(&self, id: &str) -> String {
        self.tmux(&["list-windows", "-a", "-F", "#{window_id}\t#{@ws_session}"])
            .lines()
            .find_map(|l| {
                let (w, s) = l.split_once('\t')?;
                (s == id).then(|| w.into())
            })
            .unwrap()
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
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap().trim_end().into()
}
