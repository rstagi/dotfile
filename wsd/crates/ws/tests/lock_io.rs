mod common;

use std::{
    io::{Read, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

#[test]
fn workspace_git_fetch_does_not_hold_the_session_store_lock() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("wsd.sock");
    let server = format!(
        "ws-lock-io-{}-{}",
        std::process::id(),
        home.path().file_name().unwrap().to_str().unwrap()
    );
    let real_git = Command::new("zsh")
        .args(["-fc", "command -v git"])
        .output()
        .unwrap();
    let real_git = String::from_utf8(real_git.stdout).unwrap();
    let repo = home.path().join("api");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &[
            "-c",
            "user.email=t@e.invalid",
            "-c",
            "user.name=T",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "seed",
        ],
    ] {
        assert!(
            Command::new(real_git.trim())
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let started = home.path().join("fetch-started");
    let release = home.path().join("fetch-release");
    for fifo in [&started, &release] {
        assert!(Command::new("mkfifo").arg(fifo).status().unwrap().success());
    }
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("git"),
        format!(
            "#!/bin/sh\ncase \" $* \" in *\" fetch \"*) echo started > {}; read _ < {}; exit 0 ;; esac\nexec {} \"$@\"\n",
            shell_words::quote(started.to_str().unwrap()),
            shell_words::quote(release.to_str().unwrap()),
            shell_words::quote(real_git.trim())
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = common::ws(home.path(), &socket);
    let path = command
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, value)| value)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    command
        .env("PATH", format!("{}:{path}", bin.display()))
        .env("WS_ROOT", home.path().join("worktrees"))
        .env("WS_TMUX_SOCKET", &server)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("WS_SESSION")
        .env_remove("WS_SESSION_ID");
    let path = format!("{}:{path}", bin.display());
    let _daemon = common::Daemon::start(command, &socket);
    let _tmux = TmuxServer(server.clone(), home.path().to_owned());
    let mut create = common::ws(home.path(), &socket);
    create
        .env("PATH", &path)
        .env("WS_ROOT", home.path().join("worktrees"))
        .env("WS_TMUX_SOCKET", &server)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("WS_SESSION")
        .env_remove("WS_SESSION_ID")
        .args([
            "new",
            "--repo",
            repo.to_str().unwrap(),
            "--branch",
            "feature",
            "--detach",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = create.spawn().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let watcher_started = started.clone();
    let watcher = std::thread::spawn(move || {
        let output = child.wait_with_output().unwrap();
        // Unblock the reader when `ws new` exits before it reaches `git fetch`.
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc_nonblock())
            .open(&watcher_started);
        let _ = sender.send(());
        output
    });
    // Opening the FIFO blocks until the fake git reaches `git fetch`.
    let marker = std::fs::read_to_string(&started).unwrap();
    if marker != "started\n" {
        panic!(
            "ws new exited before git fetch: {:?}",
            watcher.join().unwrap()
        );
    }
    let listed = request(&socket, "/v1/sessions", Duration::from_secs(2));
    let workspaces = request(&socket, "/v1/workspaces", Duration::from_secs(2));
    std::fs::write(&release, "go\n").unwrap();
    receiver.recv().unwrap();
    let output = watcher.join().unwrap();
    let listed = listed.expect("session list must not wait for git fetch");
    assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
    let workspaces = workspaces.expect("workspace list must not wait for git fetch");
    assert!(workspaces.starts_with("HTTP/1.1 200"), "{workspaces}");
    assert!(output.status.success(), "{output:?}");
    assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let created = common::request(&socket, "GET", "/v1/workspaces", "");
    assert!(created.contains("api/feature"), "{created}");
}

#[test]
fn headless_stdin_fifo_without_writer_does_not_hold_the_session_store_lock() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let socket = home.path().join("wsd.sock");
    let mut command = common::ws(home.path(), &socket);
    command.env_remove("WS_SESSION").env_remove("WS_SESSION_ID");
    let _daemon = common::Daemon::start(command, &socket);
    let fifo = home.path().join("stdin");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let output = home.path().join("output");
    let mut create = common::ws(home.path(), &socket);
    create
        .env_remove("WS_SESSION")
        .env_remove("WS_SESSION_ID")
        .args(["session", "create", "--headless", "--stdin"])
        .arg(&fifo)
        .arg("--stdout")
        .arg(&output)
        .args(["--", "/bin/cat"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = create.spawn().unwrap();
    let listed = request(&socket, "/v1/sessions", Duration::from_secs(2));
    if listed.is_err() {
        // Give the blocked open a writer so the daemon and the CLI can end.
        drop(std::fs::OpenOptions::new().write(true).open(&fifo));
    }
    let created = child.wait_with_output().unwrap();
    let listed = listed.expect("session list must not wait for the stdin FIFO open");
    assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
    assert!(created.status.success(), "{created:?}");
    let id = String::from_utf8(created.stdout).unwrap().trim().to_owned();
    // The command sees end of input because the FIFO has no writer.
    let waited = common::request(
        &socket,
        "POST",
        &format!("/v1/sessions/{id}/wait"),
        r#"{"for":"exit","timeout":10}"#,
    );
    assert!(waited.starts_with("HTTP/1.1 200"), "{waited}");
    assert!(waited.contains(r#""state":"finished""#), "{waited}");
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "");
}

struct TmuxServer(String, std::path::PathBuf);

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.0, "kill-server"])
            .env("HOME", &self.1)
            .output();
    }
}

fn request(socket: &Path, path: &str, timeout: Duration) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn libc_nonblock() -> i32 {
    // O_NONBLOCK on macOS and Linux differs. Use the platform value.
    if cfg!(target_os = "macos") {
        0x0004
    } else {
        0o4000
    }
}
