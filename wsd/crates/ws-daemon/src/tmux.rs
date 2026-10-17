//! Small tmux driver. Commands pass arguments directly to tmux.

use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_BUFFER: AtomicU64 = AtomicU64::new(0);
/// tmux runs a single command argument as shell text. This prefix keeps it one argv word.
const EXEC_ARGV: [&str; 4] = ["/bin/sh", "-c", "exec \"$@\"", "sh"];

pub struct Tmux {
    socket: Option<OsString>,
}

impl Tmux {
    pub fn named(socket: impl Into<OsString>) -> Self {
        Self {
            socket: Some(socket.into()),
        }
    }

    pub fn from_env() -> Self {
        Self {
            socket: env::var_os("WS_TMUX_SOCKET"),
        }
    }

    /// Creates a detached session and returns its `$N` id.
    /// An empty `command` starts the default shell.
    pub fn create_session(&self, name: &str, command: &[&str]) -> io::Result<String> {
        let wrapper: &[&str] = if command.len() == 1 { &EXEC_ARGV } else { &[] };
        output_text(
            self.command()
                .args(["new-session", "-d", "-P", "-F", "#{session_id}", "-s", name])
                .args(wrapper)
                .args(command)
                .output()?,
        )
    }

    /// Starts `command` as an argv vector. An empty `command` starts the default shell.
    pub fn create_window(&self, session: &str, name: &str, command: &[&str]) -> io::Result<String> {
        let wrapper: &[&str] = if command.len() == 1 { &EXEC_ARGV } else { &[] };
        let output = self
            .command()
            .args([
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{window_id}",
                "-t",
                session,
                "-n",
                name,
            ])
            .args(wrapper)
            .args(command)
            .output()?;
        output_text(output)
    }

    /// Splits the target vertically and returns the detached pane's `%N` id.
    /// An empty `command` starts the default shell.
    pub fn create_pane(&self, target: &str, command: &[&str]) -> io::Result<String> {
        let wrapper: &[&str] = if command.len() == 1 { &EXEC_ARGV } else { &[] };
        output_text(
            self.command()
                .args(["split-window", "-d", "-P", "-F", "#{pane_id}", "-t", target])
                .args(wrapper)
                .args(command)
                .output()?,
        )
    }

    pub fn set_state(&self, window: &str, state: &str) -> io::Result<()> {
        self.set_window_option(window, "@ws_state", state)
    }

    pub fn state(&self, window: &str) -> io::Result<String> {
        self.execute(&["show-options", "-w", "-v", "-t", window, "@ws_state"])
    }

    /// Sets a window option. Values are literal text, without format expansion.
    pub fn set_window_option(&self, window: &str, option: &str, value: &str) -> io::Result<()> {
        self.execute(&["set-option", "-w", "-t", window, option, value])?;
        Ok(())
    }

    /// Reads a window option. `None` means unset. An empty value returns `Some("")`.
    pub fn window_option(&self, window: &str, option: &str) -> io::Result<Option<String>> {
        // Validate the target first. The quiet option query also suppresses target errors.
        let output = self
            .command()
            .args([
                "display-message",
                "-p",
                "-t",
                window,
                "#{window_id}",
                ";",
                "show-options",
                "-wqv",
                "-t",
                window,
                option,
            ])
            .output()?;
        let text = output_string(output)?;
        let (id, value) = text.split_once('\n').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The tmux response has no window id.",
            )
        })?;
        if id.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "The tmux window does not exist.",
            ));
        }
        Ok(value.strip_suffix('\n').map(str::to_owned))
    }

    pub fn unset_window_option(&self, window: &str, option: &str) -> io::Result<()> {
        self.execute(&["set-option", "-wu", "-t", window, option])?;
        Ok(())
    }

    pub fn paste(&self, target: &str, text: &str) -> io::Result<()> {
        let buffer = format!(
            "ws-{}-{}",
            std::process::id(),
            NEXT_BUFFER.fetch_add(1, Ordering::Relaxed)
        );
        let mut child = self
            .command()
            .args(["load-buffer", "-b", &buffer, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let written = child.stdin.take().unwrap().write_all(text.as_bytes());
        let loaded = child.wait_with_output().and_then(output_text);
        let result = written.and(loaded.map(|_| ())).and_then(|()| {
            self.execute(&[
                "paste-buffer",
                "-d",
                "-p",
                "-r",
                "-b",
                &buffer,
                "-t",
                target,
            ])
            .map(|_| ())
        });
        if result.is_err() {
            let _ = self.execute(&["delete-buffer", "-b", &buffer]);
        }
        result
    }

    pub fn window_for_pane(&self, pane: &str) -> io::Result<String> {
        self.execute(&["display-message", "-p", "-t", pane, "#{window_id}"])
    }

    /// Captures the screen and available scrollback, with terminal colors and final newlines.
    pub fn capture_pane(&self, target: &str) -> io::Result<String> {
        output_string(
            self.command()
                .args(["capture-pane", "-p", "-e", "-S", "-", "-t", target])
                .output()?,
        )
    }

    fn execute(&self, args: &[&str]) -> io::Result<String> {
        output_text(self.command().args(args).output()?)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        if let Some(socket) = &self.socket {
            command.arg("-L").arg(socket);
        }
        command
    }
}

fn output_text(output: Output) -> io::Result<String> {
    output_string(output).map(|mut text| {
        if text.ends_with('\n') {
            text.pop();
        }
        text
    })
}

fn output_string(output: Output) -> io::Result<String> {
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "The tmux command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::Tmux;
    use std::{
        process::{Command, Output},
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    static NEXT_SERVER: AtomicU64 = AtomicU64::new(0);

    struct Sandbox {
        home: tempfile::TempDir,
        server: String,
        tmux: Tmux,
    }

    #[test]
    fn creates_detached_sessions_windows_and_panes() {
        let sandbox = Sandbox::new();
        let session = sandbox
            .tmux
            .create_session("workspace", &["/bin/sleep", "300"])
            .unwrap();
        assert!(session.starts_with('$'), "{session}");
        let window = sandbox
            .tmux
            .create_window(&session, "agent", &["/bin/sleep", "300"])
            .unwrap();
        let pane = sandbox
            .tmux
            .create_pane(&window, &["/bin/sleep", "300"])
            .unwrap();
        assert!(pane.starts_with('%'), "{pane}");
        assert_eq!(sandbox.tmux.window_for_pane(&pane).unwrap(), window);
    }

    #[test]
    fn reads_sets_and_removes_all_legacy_window_options() {
        let sandbox = Sandbox::new();
        let window = sandbox
            .tmux
            .create_window("keepalive", "options", &["/bin/sleep", "300"])
            .unwrap();
        // These names come from tag_window, hooks, bus, picker and notify in ws/lib.
        let options = [
            ("@ws_path", "/tmp/work tree"),
            ("@ws_slot", "2"),
            ("@ws_session", "123"),
            ("@ws_agent", "claude"),
            ("@ws_state", "idle"),
            ("@ws_pane", "%3"),
            ("@ws_main", "working"),
            ("@ws_turn", "1"),
            ("@ws_unread", "1"),
            ("@ws_seq", "7"),
            ("@ws_notify_open", "1"),
            ("@ws_notify_seq", "7"),
            ("@ws_sub_agent-123", "1"),
        ];
        for (option, value) in options {
            assert_eq!(sandbox.tmux.window_option(&window, option).unwrap(), None);
            sandbox
                .tmux
                .set_window_option(&window, option, value)
                .unwrap();
            assert_eq!(
                sandbox
                    .tmux
                    .window_option(&window, option)
                    .unwrap()
                    .as_deref(),
                Some(value)
            );
            sandbox.tmux.unset_window_option(&window, option).unwrap();
            assert_eq!(sandbox.tmux.window_option(&window, option).unwrap(), None);
        }
        let literal = "  'quotes' $HOME `echo unsafe` #{window_id}\n\t ";
        sandbox
            .tmux
            .set_window_option(&window, "@ws_path", literal)
            .unwrap();
        assert_eq!(
            sandbox
                .tmux
                .window_option(&window, "@ws_path")
                .unwrap()
                .as_deref(),
            Some(literal)
        );
        sandbox
            .tmux
            .set_window_option(&window, "@ws_path", "")
            .unwrap();
        assert_eq!(
            sandbox.tmux.window_option(&window, "@ws_path").unwrap(),
            Some(String::new())
        );
        assert!(sandbox.tmux.window_option("@999999", "@ws_path").is_err());
    }

    #[test]
    fn captures_pane_history_and_terminal_colors() {
        let sandbox = Sandbox::new();
        let script = "printf 'first marker\\n'; i=0; while [ $i -lt 80 ]; do printf 'history %s\\n' \"$i\"; i=$((i+1)); done; printf '\\033[31mred marker\\033[0m\\n'; exec /bin/sleep 300";
        let window = sandbox
            .tmux
            .create_window("keepalive", "capture", &["/bin/sh", "-c", script])
            .unwrap();
        wait_until(|| {
            sandbox
                .tmux
                .capture_pane(&window)
                .unwrap()
                .contains("red marker")
        });
        let captured = sandbox.tmux.capture_pane(&window).unwrap();
        assert!(captured.starts_with("first marker\n"), "{captured:?}");
        assert!(captured.contains("history 79\n"), "{captured:?}");
        assert!(captured.contains("\x1b[31mred marker"), "{captured:?}");
        assert!(captured.ends_with('\n'), "{captured:?}");
        assert!(sandbox.tmux.capture_pane("%999999").is_err());
    }

    impl Sandbox {
        fn new() -> Self {
            let server = format!(
                "ws-driver-test-{}-{}",
                std::process::id(),
                NEXT_SERVER.fetch_add(1, Ordering::Relaxed)
            );
            let sandbox = Self {
                home: tempfile::tempdir_in("/tmp").unwrap(),
                tmux: Tmux::named(&server),
                server,
            };
            sandbox.output(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "keepalive",
                "-x",
                "100",
                "-y",
                "40",
                "/bin/sleep",
                "300",
            ]);
            sandbox
        }

        fn output(&self, args: &[&str]) -> Output {
            let output = Command::new("tmux")
                .args(["-L", &self.server])
                .args(args)
                .env("HOME", self.home.path())
                .env("WS_HOME", self.home.path().join(".ws"))
                .env("WS_STATE", self.home.path().join("state"))
                .env("WS_SOCK", self.home.path().join("wsd.sock"))
                .env("WS_TMUX_SOCKET", &self.server)
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
            output
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", &self.server, "kill-server"])
                .output();
        }
    }

    fn wait_until(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(
                Instant::now() < deadline,
                "The sandbox operation timed out."
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
