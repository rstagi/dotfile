use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
mod common;
use common::{Daemon, request, ws};

struct Sandbox {
    home: tempfile::TempDir,
    server: String,
    socket: PathBuf,
    pane: String,
    window: String,
    id: String,
    daemon: Option<Daemon>,
    signals: AdapterSignals,
}

struct AdapterSignals {
    receiver: mpsc::Receiver<()>,
    pipe: fs::File,
    reader: Option<thread::JoinHandle<()>>,
}

impl Sandbox {
    fn new() -> Self {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let signals = AdapterSignals::new(home.path());
        let server = format!("ws-notify-{}", ws_core::ids::Ulid::new());
        let socket = home.path().join("wsd.sock");
        let output = Command::new("tmux")
            .args([
                "-L",
                &server,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "notify",
                "/bin/sleep",
                "300",
            ])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let tmux = ws_daemon::tmux::Tmux::named(&server);
        let window = tmux
            .execute(&["display-message", "-p", "-t", "notify", "#{window_id}"])
            .unwrap();
        let pane = tmux
            .execute(&["display-message", "-p", "-t", &window, "#{pane_id}"])
            .unwrap();
        let _ = ws(home.path(), &socket);
        for name in ["terminal-notifier", "osascript", "afplay"] {
            let path = home.path().join("fake-agents").join(name);
            fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/{name}.tmp\"\nmv \"$HOME/{name}.tmp\" \"$HOME/{name}.out\"\nprintf . > \"$HOME/adapter-ready\"\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut command = ws(home.path(), &socket);
        command.env("WS_TMUX_SOCKET", &server).env("WS_NOTIFY", "1");
        let daemon = Daemon::start(command, &socket);
        assert!(common::get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
        let created = request(&socket, "POST", "/v1/sessions", "{}");
        let created: Value =
            serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        for (key, value) in [
            ("@ws_session", id.as_str()),
            ("@ws_pane", &pane),
            ("@ws_agent", "claude"),
            ("@ws_state", "working"),
            ("@ws_path", home.path().to_str().unwrap()),
        ] {
            tmux.set_window_option(&window, key, value).unwrap();
        }
        Self {
            home,
            server,
            socket,
            pane,
            window,
            id,
            daemon: Some(daemon),
            signals,
        }
    }

    fn command(&self) -> Command {
        let mut command = ws(self.home.path(), &self.socket);
        command
            .env("WS_TMUX_SOCKET", &self.server)
            .env("TMUX_PANE", &self.pane)
            .env("WS_SESSION_ID", &self.id)
            .env("WS_WORKSPACE", "api/feat-login")
            .env("WS_NOTIFY", "1")
            .env("WS_NOTIFIER", "terminal-notifier")
            .env("WS_TERMINAL_BUNDLE", "com.example.term");
        command
    }

    fn hook(&self, state: &str) {
        let mut child = self
            .command()
            .args(["hook", state])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"{}").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    fn output(&self, adapter: &str) -> String {
        let path = self.home.path().join(format!("{adapter}.out"));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(text) = fs::read_to_string(&path) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "No {adapter} output. Daemon: {}",
                self.daemon.as_ref().unwrap().stderr()
            );
            self.signals.wait(deadline);
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        self.daemon.take();
        let _ = Command::new("tmux")
            .args(["-L", &self.server, "kill-server"])
            .output();
    }
}

#[test]
fn unfocused_waiting_hook_preserves_terminal_notifier_arguments() {
    let sandbox = Sandbox::new();
    sandbox.hook("waiting");
    let output = sandbox.output("terminal-notifier");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(
        &lines[..6],
        [
            "-title",
            "ws",
            "-message",
            "api/feat-login needs input",
            "-group",
            &format!("ws-{}", sandbox.window)
        ]
    );
    assert!(
        output.contains("-activate\ncom.example.term\n-execute\n"),
        "{output}"
    );
    assert!(output.contains("select-window"));
    assert!(output.contains("switch-client"));
    assert!(output.contains(&sandbox.server));
    assert!(output.contains(&sandbox.window));
    assert!(!output.contains("-sound"));
}

#[test]
fn explicit_osascript_and_failed_terminal_notifier_use_the_legacy_command_line() {
    for adapter in ["osascript", "terminal-notifier"] {
        let sandbox = Sandbox::new();
        let notifier = sandbox.home.path().join("fake-agents/terminal-notifier");
        fs::write(notifier, "#!/bin/sh\nexit 1\n").unwrap();
        let output = sandbox
            .command()
            .env("WS_NOTIFIER", adapter)
            .args(["hook", "waiting"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            sandbox.output("osascript"),
            "-e\non run argv\n-e\ndisplay notification (item 1 of argv) with title (item 2 of argv)\n-e\nend run\napi/feat-login needs input\nws\n"
        );
    }
}

#[test]
fn custom_adapter_receives_the_zsh_events_and_dismisses_once() {
    let sandbox = Sandbox::new();
    let repo = sandbox.home.path().join("api/feat-login");
    fs::create_dir_all(&repo).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-b", "feat-login"])
            .arg(&repo)
            .output()
            .unwrap()
            .status
            .success()
    );
    let tmux = ws_daemon::tmux::Tmux::named(&sandbox.server);
    tmux.set_window_option(&sandbox.window, "@ws_path", repo.to_str().unwrap())
        .unwrap();
    tmux.set_state(&sandbox.window, "idle").unwrap();
    let adapter = sandbox.home.path().join("custom adapter");
    fs::write(
        &adapter,
        "#!/bin/sh\ncat >> \"$HOME/events.jsonl\"\nprintf . > \"$HOME/adapter-ready\"\n",
    )
    .unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    for (state, payload, count) in [
        ("working", "{}", 1),
        ("waiting", r#"{"message":"Need permission"}"#, 2),
        ("waiting", r#"{"message":"Duplicate"}"#, 2),
        ("working", "{}", 3),
        ("idle", r#"{"last_assistant_message":"Done"}"#, 4),
    ] {
        let mut child = sandbox
            .command()
            .env("WS_NOTIFIER", &adapter)
            .args(["hook", state])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        sandbox.events(count);
    }
    let output = sandbox
        .command()
        .env("WS_NOTIFIER", &adapter)
        .args(["_dismiss", &sandbox.window])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    sandbox.events(5);
    let output = sandbox
        .command()
        .env("WS_NOTIFIER", &adapter)
        .args(["_dismiss", &sandbox.window])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let events = sandbox.events(5);
    let actual: Vec<_> = events.iter().map(project_event).collect();
    let expected: Vec<Value> = include_str!("golden/notify-events.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(actual, expected);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["v"], 1);
        assert_eq!(event["id"], format!("ws:{}", sandbox.window));
        assert_eq!(event["source"], "ws");
        assert_eq!(event["title"], "ws");
        assert_eq!(event["actions"][0]["label"], "Focus tab");
        assert!(event["ts"].as_u64().unwrap() > 0);
        if index > 0 {
            assert!(event["seq"].as_u64().unwrap() > events[index - 1]["seq"].as_u64().unwrap());
        }
    }
    assert_eq!(tmux.state(&sandbox.window).unwrap(), "idle");
    assert_eq!(
        tmux.window_option(&sandbox.window, "@ws_notify_open")
            .unwrap()
            .as_deref(),
        Some("0")
    );
    assert!(!sandbox.home.path().join("terminal-notifier.out").exists());
    assert!(!sandbox.home.path().join("afplay.out").exists());
}

impl Sandbox {
    fn events(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let events: Vec<Value> = fs::read_to_string(self.home.path().join("events.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if events.len() >= count {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "Expected {count} notification events, got {events:?}"
            );
            self.signals.wait(deadline);
        }
    }
}

fn project_event(event: &Value) -> Value {
    let mut projected = serde_json::Map::new();
    for key in [
        "state", "prev", "message", "detail", "agent", "repo", "branch", "focused", "sound",
    ] {
        projected.insert(key.into(), event[key].clone());
    }
    Value::Object(projected)
}

impl AdapterSignals {
    fn new(home: &std::path::Path) -> Self {
        let path = home.join("adapter-ready");
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        // The fixture starts children, so neither pipe descriptor can cross a fork.
        let pipe = ws_core::lockfile::open_lock_file(&path).unwrap();
        let mut input = ws_core::lockfile::open_lock_file(&path).unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut byte = [0];
            while input.read_exact(&mut byte).is_ok() && byte != [0] {
                if sender.send(()).is_err() {
                    break;
                }
            }
        });
        Self {
            receiver,
            pipe,
            reader: Some(reader),
        }
    }

    fn wait(&self, deadline: Instant) {
        // The capture writes a byte only after the complete output is available.
        // Block here so concurrent tests do not starve daemon subprocesses.
        let _ = self
            .receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

impl Drop for AdapterSignals {
    fn drop(&mut self) {
        let _ = self.pipe.write_all(&[0]);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn boringnotch_uses_nc_with_the_event_on_stdin_and_falls_back_on_failure() {
    for success in [true, false] {
        let sandbox = Sandbox::new();
        let nc = sandbox.home.path().join("fake-agents/nc");
        fs::write(&nc, format!("#!/bin/sh\ncat > \"$HOME/notch-event.json\"\nprintf '%s\\n' \"$@\" > \"$HOME/nc.tmp\"\nmv \"$HOME/nc.tmp\" \"$HOME/nc.out\"\nprintf . > \"$HOME/adapter-ready\"\nexit {}\n", if success { 0 } else { 1 })).unwrap();
        fs::set_permissions(nc, fs::Permissions::from_mode(0o755)).unwrap();
        let socket = sandbox.home.path().join("notch notify.sock");
        let output = sandbox
            .command()
            .env("WS_NOTIFIER", "boringnotch")
            .env("WS_BORINGNOTCH_SOCKET", &socket)
            .args(["hook", "waiting"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            sandbox.output("nc"),
            format!("-U\n-w\n1\n{}\n", socket.display())
        );
        let event: Value = serde_json::from_str(
            &fs::read_to_string(sandbox.home.path().join("notch-event.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(event["state"], "waiting");
        assert_eq!(event["message"], "api/feat-login needs input");
        if success {
            assert!(!sandbox.home.path().join("terminal-notifier.out").exists());
            assert!(!sandbox.home.path().join("afplay.out").exists());
        } else {
            assert!(
                sandbox
                    .output("terminal-notifier")
                    .contains("api/feat-login needs input")
            );
            assert!(sandbox.output("afplay").contains("waiting.wav"));
        }
    }
}

#[test]
fn hook_overrides_disable_every_adapter_even_when_the_daemon_enables_notifications() {
    let sandbox = Sandbox::new();
    let adapter = sandbox.home.path().join("capture");
    fs::write(
        &adapter,
        "#!/bin/sh\ncat >> \"$HOME/events.jsonl\"\nprintf . > \"$HOME/adapter-ready\"\n",
    )
    .unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    for notifier in [
        "terminal-notifier",
        "osascript",
        "boringnotch",
        adapter.to_str().unwrap(),
    ] {
        for state in ["waiting", "idle"] {
            let output = sandbox
                .command()
                .env("WS_NOTIFY", "0")
                .env("WS_NOTIFIER", notifier)
                .args(["hook", state])
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success());
        }
    }
    assert_eq!(
        ws_daemon::tmux::Tmux::named(&sandbox.server)
            .window_option(&sandbox.window, "@ws_notify_open")
            .unwrap(),
        None
    );
    let output = sandbox
        .command()
        .env("WS_NOTIFIER", &adapter)
        .args(["hook", "waiting"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    sandbox.events(1);
    let output = sandbox
        .command()
        .env("WS_NOTIFY", "0")
        .args(["_dismiss", &sandbox.window])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = sandbox
        .command()
        .env("WS_NOTIFIER", &adapter)
        .args(["_dismiss", &sandbox.window])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        sandbox
            .events(2)
            .iter()
            .map(|event| event["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["waiting", "closed"]
    );
    for file in ["terminal-notifier.out", "osascript.out", "afplay.out"] {
        assert!(!sandbox.home.path().join(file).exists(), "{file}");
    }
}

#[test]
fn built_in_dismissal_removes_the_banner_without_changing_the_tab_state() {
    let sandbox = Sandbox::new();
    sandbox.hook("waiting");
    sandbox.output("terminal-notifier");
    fs::remove_file(sandbox.home.path().join("terminal-notifier.out")).unwrap();
    let output = sandbox
        .command()
        .args(["_dismiss", &sandbox.pane])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        sandbox.output("terminal-notifier"),
        format!("-remove\nws-{}\n", sandbox.window)
    );
    assert_eq!(
        ws_daemon::tmux::Tmux::named(&sandbox.server)
            .state(&sandbox.window)
            .unwrap(),
        "waiting"
    );
    for args in [vec!["_dismiss"], vec!["_dismiss", "@999999999"]] {
        assert!(
            sandbox
                .command()
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn relative_sound_paths_use_the_hook_working_directory() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.home.path().join("custom done.wav"), "sound").unwrap();
    let output = sandbox
        .command()
        .current_dir(sandbox.home.path())
        .env("WS_NOTIFY_SOUND_DONE", "custom done.wav")
        .args(["hook", "idle"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        sandbox
            .output("terminal-notifier")
            .contains("api/feat-login is idle")
    );
    assert_eq!(sandbox.output("afplay"), "custom done.wav\n");
}

#[test]
#[cfg(target_os = "macos")]
fn build_notifier_brands_a_copy_and_uses_it_for_notifications() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("source/terminal-notifier.app");
    fs::create_dir_all(source.join("Contents/MacOS")).unwrap();
    fs::create_dir_all(source.join("Contents/Resources")).unwrap();
    let plist = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>original.notifier</string><key>CFBundleName</key><string>terminal-notifier</string></dict></plist>";
    fs::write(source.join("Contents/Info.plist"), plist).unwrap();
    let binary = source.join("Contents/MacOS/terminal-notifier");
    fs::write(&binary, "#!/bin/sh\n{ printf 'BRANDED\\n'; printf '%s\\n' \"$@\"; } > \"$HOME/terminal-notifier.tmp\"\nmv \"$HOME/terminal-notifier.tmp\" \"$HOME/terminal-notifier.out\"\nprintf . > \"$HOME/adapter-ready\"\n").unwrap();
    fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).unwrap();
    for (name, script) in [
        (
            "sips",
            "for last; do :; done\nprintf png > \"$last\"\nprintf '%s\\n' \"$@\" >> \"$HOME/sips.out\"",
        ),
        (
            "iconutil",
            "for last; do :; done\nprintf icns > \"$last\"\nprintf '%s\\n' \"$@\" > \"$HOME/iconutil.out\"",
        ),
        ("codesign", "printf '%s\\n' \"$@\" > \"$HOME/codesign.out\""),
    ] {
        let path = sandbox.home.path().join("fake-agents").join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = sandbox
        .command()
        .env("WS_NOTIFIER_SOURCE", &source)
        .arg("_build-notifier")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let app = sandbox.home.path().join(".ws/ws.app");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("built {}\n", app.display())
    );
    assert_eq!(
        fs::read_to_string(source.join("Contents/Info.plist")).unwrap(),
        plist
    );
    let branded = fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    assert!(branded.contains("sh.ratel.ws.notifier"));
    assert!(branded.contains("<string>ws</string>"));
    assert_eq!(
        fs::read(app.join("Contents/Resources/Terminal.icns")).unwrap(),
        b"icns"
    );
    assert_eq!(
        sandbox.output("codesign"),
        format!("--force\n--deep\n--sign\n-\n{}\n", app.display())
    );
    let sips = sandbox.output("sips");
    assert_eq!(sips.lines().filter(|line| *line == "-z").count(), 10);
    sandbox.hook("waiting");
    assert!(
        sandbox
            .output("terminal-notifier")
            .starts_with("BRANDED\n-title\nws\n")
    );
}

#[test]
fn focused_tabs_send_custom_events_and_suppress_builtin_banners_and_sounds() {
    let sandbox = Sandbox::new();
    let real_tmux = Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .unwrap();
    let real_tmux = String::from_utf8(real_tmux.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let fake_tmux = sandbox.home.path().join("fake-agents/tmux");
    fs::write(&fake_tmux, format!("#!/usr/bin/env python3\nimport os, sys\nprogram = {}\nargs = [arg.replace('#{{&&:#{{window_active}},#{{session_attached}}}}', '1') for arg in sys.argv[1:]]\nos.execv(program, [program] + args)\n", serde_json::to_string(&real_tmux).unwrap())).unwrap();
    fs::set_permissions(fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    for adapter in ["auto", "terminal-notifier", "osascript"] {
        for state in ["waiting", "working", "idle"] {
            let output = sandbox
                .command()
                .env("WS_NOTIFIER", adapter)
                .args(["hook", state])
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success());
        }
    }
    let adapter = sandbox.home.path().join("capture");
    fs::write(
        &adapter,
        "#!/bin/sh\ncat >> \"$HOME/events.jsonl\"\nprintf . > \"$HOME/adapter-ready\"\n",
    )
    .unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    let output = sandbox
        .command()
        .env("WS_NOTIFIER", &adapter)
        .args(["hook", "waiting"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    let events = sandbox.events(1);
    assert_eq!(events[0]["focused"], true);
    for name in ["terminal-notifier", "osascript", "afplay"] {
        assert!(
            !sandbox.home.path().join(format!("{name}.out")).exists(),
            "{name}"
        );
    }
}
