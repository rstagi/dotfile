#[allow(dead_code)]
pub mod sandbox;
#[allow(unused_imports)]
pub use sandbox::SandboxRoot;

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Daemon {
    pub child: Child,
    stderr: tempfile::NamedTempFile,
}

#[allow(dead_code)]
pub fn latency_assertions_enabled() -> bool {
    std::env::var("WS_ASSERT_LATENCY").as_deref() == Ok("1")
}

impl Daemon {
    pub fn start(command: Command, socket: &Path) -> Self {
        let mut daemon = Self::spawn(command);
        daemon.wait_ready(socket);
        daemon
    }

    pub fn spawn(mut command: Command) -> Self {
        let stderr = tempfile::NamedTempFile::new_in("/tmp").unwrap();
        let child = command
            .arg("daemon")
            .stdout(Stdio::null())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .unwrap();
        Self { child, stderr }
    }

    pub fn wait_ready(&mut self, socket: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("The daemon exited with {status}: {}", self.stderr());
            }
            if try_request(socket, "GET", "/v1/ping", "", Duration::from_millis(200))
                .is_ok_and(|response| response.starts_with("HTTP/1.1 200 OK\r\n"))
            {
                return;
            }
            assert!(Instant::now() < deadline, "The daemon did not answer ping.");
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn stderr(&self) -> String {
        std::fs::read_to_string(self.stderr.path()).unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if thread::panicking() {
            eprintln!("Daemon {} stderr:\n{}", self.child.id(), self.stderr());
        }
    }
}

pub fn ws(home: &Path, socket: &Path) -> Command {
    use std::os::unix::fs::PermissionsExt;
    let agents = home.join("fake-agents");
    std::fs::create_dir_all(&agents).unwrap();
    for agent in ["claude", "codex", "grok", "launchctl"] {
        let path = agents.join(agent);
        if !path.exists() {
            std::fs::write(
                &path,
                if agent == "launchctl" {
                    "#!/bin/sh\nexit 0\n"
                } else {
                    "#!/bin/sh\nexec /bin/cat\n"
                },
            )
            .unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    std::fs::create_dir_all(home.join(".ws")).unwrap();
    let selector = home.join(".ws/backend");
    if !selector.exists() {
        std::fs::write(selector, "rust\n").unwrap();
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_ws"));
    command
        .env(
            "PATH",
            format!("{}:{}", agents.display(), std::env::var("PATH").unwrap()),
        )
        .env("WS_AGENT_SHELL", "zsh -fc")
        .env("GROK_HOME", home.join(".grok"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"));
    command
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", home)
        .env("WS_HOME", home.join(".ws"))
        .env("WS_ROOT", home.join(".ws/worktrees"))
        .env("WS_NOTIFY", "0")
        .env("WS_STATE", home.join("state"))
        .env("WS_SOCK", socket)
        .env("WS_TMUX_SOCKET", home.join("tmux.sock"));
    command
}

pub fn get(socket: &Path, path: &str) -> String {
    request(socket, "GET", path, "")
}

pub fn request(socket: &Path, method: &str, path: &str, body: &str) -> String {
    // The daemon completes trigger actions before it sends the response.
    // Allow parallel tests to finish that work within a generous hang guard.
    try_request(socket, method, path, body, Duration::from_secs(30)).unwrap()
}

// The tmux test binary shares this module but does not create stale sockets.
#[allow(dead_code)]
pub fn create_stale_socket(socket: &Path) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "common::stale_socket_helper",
            "--test-threads=1",
        ])
        .env("WS_TEST_STALE_SOCKET", socket)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(socket.exists());
}

#[test]
#[ignore = "Creates a stale socket in a separate process."]
fn stale_socket_helper() {
    let socket = std::env::var_os("WS_TEST_STALE_SOCKET").unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
}

fn try_request(
    socket: &Path,
    method: &str,
    path: &str,
    body: &str,
    timeout: Duration,
) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}
