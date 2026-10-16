//! Small tmux driver. Commands pass arguments directly to tmux.

use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_BUFFER: AtomicU64 = AtomicU64::new(0);

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

    pub fn create_window(&self, session: &str, name: &str, command: &[&str]) -> io::Result<String> {
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
            .args(command)
            .output()?;
        output_text(output)
    }

    pub fn set_state(&self, window: &str, state: &str) -> io::Result<()> {
        self.execute(&["set-option", "-w", "-t", window, "@ws_state", state])?;
        Ok(())
    }

    pub fn state(&self, window: &str) -> io::Result<String> {
        self.execute(&["show-options", "-w", "-v", "-t", window, "@ws_state"])
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
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "The tmux command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim_end().to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
