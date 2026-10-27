use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

#[allow(dead_code)]
mod common;

struct Client(Child);

struct BackgroundDaemon {
    binary: std::path::PathBuf,
}

#[test]
fn ping_starts_a_background_daemon_for_a_missing_socket() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let (_cleanup, mut command) = cli(home.path(), false);
    let output = run(command.arg("ping"));
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(common::get(&home.path().join("wsd.sock"), "/v1/ping").starts_with("HTTP/1.1 200"));
    let calls = fs::read_to_string(home.path().join("launchctl.calls")).unwrap();
    assert!(calls.contains("print gui/"), "{calls}");
    assert!(calls.contains("/com.rstagi.wsd"), "{calls}");
    assert!(!calls.contains("kickstart"), "{calls}");
    assert!(home.path().join("wsd.daemon.log").exists());
}

#[test]
fn ping_does_not_start_another_daemon_when_launchd_owns_it() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let (_cleanup, mut command) = cli(home.path(), true);
    let output = run(command.arg("ping"));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!home.path().join("wsd.sock").exists());
    let calls = fs::read_to_string(home.path().join("launchctl.calls")).unwrap();
    assert!(calls.contains("/com.rstagi.wsd"), "{calls}");
}

#[test]
fn ping_kickstarts_the_launchd_job_when_its_plist_exists() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let (_cleanup, mut command) = cli(home.path(), false);
    let plist = home
        .path()
        .join("Library/LaunchAgents/com.rstagi.wsd.plist");
    fs::create_dir_all(plist.parent().unwrap()).unwrap();
    fs::write(plist, "<plist/>").unwrap();
    // The fake job answers `kickstart` as launchd does: it starts the daemon outside the client.
    let launchctl = home.path().join("fake-agents/launchctl");
    fs::write(
        &launchctl,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/launchctl.calls\"\n\
             if [ \"$1\" = kickstart ]; then\n\
             nohup {} daemon >> \"$HOME/job.log\" 2>&1 < /dev/null &\nfi\nexit 0\n",
            home.path().join("ws-test").display()
        ),
    )
    .unwrap();
    let output = run(command.arg("ping"));
    assert!(output.status.success(), "{output:?}");
    assert!(common::get(&home.path().join("wsd.sock"), "/v1/ping").starts_with("HTTP/1.1 200"));
    let calls = fs::read_to_string(home.path().join("launchctl.calls")).unwrap();
    assert!(
        calls
            .lines()
            .any(|line| line.starts_with("kickstart gui/") && line.ends_with("/com.rstagi.wsd")),
        "{calls}"
    );
    assert!(!calls.contains("print "), "{calls}");
    assert!(
        !home.path().join("wsd.daemon.log").exists(),
        "The client started its own daemon."
    );
}

#[test]
fn concurrent_cli_calls_share_one_background_daemon() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let (_cleanup, command) = cli(home.path(), false);
    let mut commands: Vec<_> = (0..4)
        .map(|_| {
            let mut child = Command::new(command.get_program());
            for (key, value) in command.get_envs() {
                if let Some(value) = value {
                    child.env(key, value);
                } else {
                    child.env_remove(key);
                }
            }
            child.arg("ping");
            child
        })
        .collect();
    thread::scope(|scope| {
        let clients: Vec<_> = commands
            .iter_mut()
            .map(|command| scope.spawn(|| run(command)))
            .collect();
        for client in clients {
            let output = client.join().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    });
    assert!(common::get(&home.path().join("wsd.sock"), "/v1/ping").starts_with("HTTP/1.1 200"));
}

#[test]
fn a_failed_background_start_reports_its_log_and_leaves_no_daemon() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let (_cleanup, mut command) = cli(home.path(), false);
    fs::create_dir_all(home.path().join("state")).unwrap();
    let lock = ws_core::lockfile::open_lock_file(&home.path().join("state/sessions.lock")).unwrap();
    lock.try_lock().unwrap();
    let output = run(command.arg("ping"));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("The daemon exited"), "{diagnostic}");
    assert!(diagnostic.contains("daemon.log"), "{diagnostic}");
    assert!(!home.path().join("wsd.sock").exists());
}

fn run(command: &mut Command) -> Output {
    let stdout = tempfile::NamedTempFile::new_in("/tmp").unwrap();
    let stderr = tempfile::NamedTempFile::new_in("/tmp").unwrap();
    let mut client = Client(
        command
            .stdin(Stdio::null())
            .stdout(stdout.reopen().unwrap())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = client.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "The CLI did not exit.");
        thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: fs::read(stdout.path()).unwrap(),
        stderr: fs::read(stderr.path()).unwrap(),
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for BackgroundDaemon {
    fn drop(&mut self) {
        // The copied binary identifies only this test's processes, including failed starts.
        let output = Command::new("pgrep")
            .args(["-f", &format!("^{} daemon$", self.binary.display())])
            .output()
            .unwrap();
        for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            let _ = Command::new("kill").args(["-KILL", pid]).status();
        }
    }
}

fn cli(home: &Path, owned: bool) -> (BackgroundDaemon, Command) {
    let socket = home.join("wsd.sock");
    let mut command = common::ws(home, &socket);
    let launchctl = home.join("fake-agents/launchctl");
    fs::write(
        &launchctl,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOME/launchctl.calls\"\nexit {}\n",
            if owned { 0 } else { 1 }
        ),
    )
    .unwrap();
    fs::set_permissions(launchctl, fs::Permissions::from_mode(0o755)).unwrap();
    let binary = home.join("ws-test");
    fs::copy(env!("CARGO_BIN_EXE_ws"), &binary).unwrap();
    let binary = binary.canonicalize().unwrap();
    let mut copied = Command::new(&binary);
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            copied.env(key, value);
        } else {
            copied.env_remove(key);
        }
    }
    command = copied;
    (BackgroundDaemon { binary }, command)
}
