use std::{
    fs,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use ws_daemon::tmux::Tmux;

struct Sandbox {
    home: tempfile::TempDir,
    server: String,
}

#[test]
fn sandbox_window_stores_state_and_receives_bracketed_paste() {
    let sandbox = Sandbox::new();
    let tmux = Tmux::named(&sandbox.server);
    let captured = sandbox.home.path().join("paste.bin");
    let script = r#"
import os, sys, tty
tty.setraw(0)
sys.stdout.write('\x1b[?2004h')
sys.stdout.flush()
data = b''
while not data.endswith(b'\x1b[201~'):
    data += os.read(0, 4096)
with open(sys.argv[1], 'wb') as f:
    f.write(data)
"#;
    let window = tmux
        .create_window(
            "spike",
            "paste",
            &["python3", "-c", script, captured.to_str().unwrap()],
        )
        .unwrap();
    assert!(window.starts_with('@'), "{window}");
    tmux.set_state(&window, "working").unwrap();
    assert_eq!(tmux.state(&window).unwrap(), "working");

    wait_until(|| {
        sandbox.text(&[
            "display-message",
            "-p",
            "-t",
            &window,
            "#{bracket_paste_flag}",
        ]) == "1"
    });
    let text = "literal 'quotes' $HOME `echo unsafe`\nsecond line";
    tmux.paste(&window, text).unwrap();
    wait_until(|| captured.exists());
    assert_eq!(
        fs::read(captured).unwrap(),
        format!("\x1b[200~{text}\x1b[201~").as_bytes()
    );
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            home: tempfile::tempdir_in("/tmp").unwrap(),
            server: format!("ws-rs-test-{}", std::process::id()),
        };
        sandbox.output(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "spike",
            "-n",
            "keepalive",
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
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        output
    }

    fn text(&self, args: &[&str]) -> String {
        String::from_utf8(self.output(args).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "The sandbox operation timed out."
        );
        thread::sleep(Duration::from_millis(5));
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.server, "kill-server"])
            .env("HOME", self.home.path())
            .output();
    }
}
