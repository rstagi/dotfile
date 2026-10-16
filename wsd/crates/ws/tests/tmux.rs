use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use ws_daemon::tmux::Tmux;

struct Sandbox {
    home: tempfile::TempDir,
    server: String,
}

struct Daemon(Child);

#[test]
fn sandbox_tmux_driver_and_stop_hook_round_trip() {
    let sandbox = Sandbox::new();
    let tmux = Tmux::named(&sandbox.server);
    let socket = sandbox.home.path().join("wsd.sock");
    let _daemon = Daemon::start(&sandbox, &socket);
    let captured = sandbox.home.path().join("paste.bin");
    let measurements = sandbox.home.path().join("latency.json");
    let script = r#"
import json, os, subprocess, sys, time, tty
tty.setraw(0)
sys.stdout.write('\x1b[?2004h')
sys.stdout.flush()
data = b''
while not data.endswith(b'\x1b[201~'):
    data += os.read(0, 4096)
with open(sys.argv[1] + '.tmp', 'wb') as f:
    f.write(data)
os.replace(sys.argv[1] + '.tmp', sys.argv[1])
latencies = []
errors = []
for _ in range(100):
    started = time.monotonic_ns()
    result = subprocess.run([sys.argv[2], 'hook', 'stop'], capture_output=True, timeout=5)
    latencies.append((time.monotonic_ns() - started) / 1e6)
    if result.returncode:
        errors.append(result.stderr.decode())
        break
with open(sys.argv[3] + '.tmp', 'w') as f:
    json.dump({'latencies': latencies, 'errors': errors}, f)
os.replace(sys.argv[3] + '.tmp', sys.argv[3])
"#;
    let window = tmux
        .create_window(
            "spike",
            "paste",
            &[
                "python3",
                "-c",
                script,
                captured.to_str().unwrap(),
                env!("CARGO_BIN_EXE_ws"),
                measurements.to_str().unwrap(),
            ],
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

    wait_until(|| measurements.exists());
    let measured: serde_json::Value =
        serde_json::from_slice(&fs::read(measurements).unwrap()).unwrap();
    assert_eq!(measured["errors"], serde_json::json!([]), "{measured}");
    let records = get_hooks(&socket);
    assert_eq!(records.as_array().unwrap().len(), 100, "{records}");
    for record in records.as_array().unwrap() {
        assert_eq!(record["event"], "stop");
        assert_eq!(record["window_id"], window);
    }
    let mut latencies: Vec<f64> = serde_json::from_value(measured["latencies"].clone()).unwrap();
    assert_eq!(latencies.len(), 100);
    latencies.sort_by(f64::total_cmp);
    println!(
        "HOOK_ROUND_TRIP samples=100 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        latencies[49], latencies[94], latencies[99]
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

    fn text(&self, args: &[&str]) -> String {
        String::from_utf8(self.output(args).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }
}

impl Daemon {
    fn start(sandbox: &Sandbox, socket: &Path) -> Self {
        let mut daemon = Self(
            Command::new(env!("CARGO_BIN_EXE_ws"))
                .arg("daemon")
                .env("HOME", sandbox.home.path())
                .env("WS_HOME", sandbox.home.path().join(".ws"))
                .env("WS_STATE", sandbox.home.path().join("state"))
                .env("WS_SOCK", socket)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_until(|| {
            assert!(daemon.0.try_wait().unwrap().is_none(), "The daemon exited.");
            UnixStream::connect(socket).is_ok()
        });
        daemon
    }
}

fn get_hooks(socket: &Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET /v1/hooks HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
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

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
