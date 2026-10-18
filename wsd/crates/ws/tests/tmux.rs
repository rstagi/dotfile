use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use ws_daemon::tmux::Tmux;

mod common;
use common::{Daemon, get, request, ws};

struct Sandbox {
    home: common::SandboxRoot,
    server: String,
}

#[test]
fn latency_assertions_require_exactly_one() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    for (value, expected) in [
        (None, "0"),
        (Some(OsString::from("")), "0"),
        (Some(OsString::from("0")), "0"),
        (Some(OsString::from("1")), "1"),
        (Some(OsString::from("true")), "0"),
        (Some(OsString::from("01")), "0"),
        (Some(OsString::from("1 ")), "0"),
        (Some(OsString::from_vec(vec![0xff])), "0"),
    ] {
        let mut probe = Command::new(std::env::current_exe().unwrap());
        probe
            .args(["--ignored", "--exact", "latency_gate_probe"])
            .env_remove("WS_ASSERT_LATENCY")
            .env("WS_TEST_EXPECT_LATENCY", expected);
        if let Some(value) = &value {
            probe.env("WS_ASSERT_LATENCY", value);
        }
        let output = probe.output().unwrap();
        assert!(output.status.success(), "{value:?}: {output:?}");
    }
}

#[test]
#[ignore = "Checks the latency gate in an isolated process."]
fn latency_gate_probe() {
    assert_eq!(
        common::latency_assertions_enabled(),
        std::env::var("WS_TEST_EXPECT_LATENCY").unwrap() == "1"
    );
}

#[test]
fn driver_uses_an_explicit_socket_path_and_sandbox_cleans_up_after_panic() {
    let home = common::SandboxRoot::new();
    let directory = home.path().to_owned();
    let socket = home.tmux_socket();
    let started = Command::new("tmux")
        .args([
            "-S",
            &socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "keepalive",
            "/bin/sleep",
            "300",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    let tmux = Tmux::named(&socket);
    let session = tmux
        .create_session("isolated", &["/bin/sleep", "300"])
        .unwrap();
    assert!(session.starts_with('$'));
    let pid = tmux.execute(&["display-message", "-p", "#{pid}"]).unwrap();
    let result = std::panic::catch_unwind(move || {
        let _home = home;
        panic!("Test sandbox unwinding.");
    });
    assert!(result.is_err());
    assert!(!directory.exists());
    let deadline = Instant::now() + Duration::from_secs(30);
    while Command::new("kill")
        .args(["-0", &pid])
        .output()
        .unwrap()
        .status
        .success()
    {
        assert!(
            Instant::now() < deadline,
            "The test tmux server did not stop."
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn sandbox_tmux_driver_and_stop_hook_round_trip() {
    let sandbox = Sandbox::new();
    let tmux = Tmux::named(&sandbox.server);
    let socket = sandbox.home.path().join("wsd.sock");
    let _daemon = Daemon::start(ws(sandbox.home.path(), &socket), &socket);
    assert!(get(&socket, "/v1/ping").starts_with("HTTP/1.1 200"));
    let created = request(&socket, "POST", "/v1/sessions", "{}");
    let created: serde_json::Value =
        serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
    let id = created["id"].as_str().unwrap();
    let captured = sandbox.home.path().join("paste.bin");
    let measurements = sandbox.home.path().join("latency.json");
    let script = r#"
import json, os, subprocess, sys, time, tty
os.environ["WS_SESSION_ID"] = sys.argv[4]
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
                id,
            ],
        )
        .unwrap();
    assert!(window.starts_with('@'), "{window}");
    tmux.set_window_option(&window, "remain-on-exit", "on")
        .unwrap();
    tmux.set_window_option(&window, "@ws_session", id).unwrap();
    let pane = tmux
        .execute(&["display-message", "-p", "-t", &window, "#{pane_id}"])
        .unwrap();
    tmux.set_window_option(&window, "@ws_pane", &pane).unwrap();
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
    let log_path = sandbox
        .home
        .path()
        .join("state/sessions")
        .join(id)
        .join("log.jsonl");
    let mut records: Vec<serde_json::Value> = Vec::new();
    // A timed-out client can finish before the daemon records its callback.
    wait_until(|| {
        records = fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        ["hook.received", "turn.ended"].iter().all(|kind| {
            records
                .iter()
                .filter(|record| record["type"] == *kind)
                .count()
                == 100
        })
    });
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "hook.received")
            .count(),
        100
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "turn.ended")
            .count(),
        100
    );
    assert_eq!(tmux.state(&window).unwrap(), "idle");
    let spool = sandbox.home.path().join("state/spool");
    if spool.exists() {
        for file in fs::read_dir(&spool).unwrap() {
            let path = file.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let callback: serde_json::Value =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                assert!(
                    records
                        .iter()
                        .any(|record| record["type"] == "hook.received"
                            && record["body"]["callback_id"] == callback["callback_id"])
                );
            }
        }
    }
    let mut latencies: Vec<f64> = serde_json::from_value(measured["latencies"].clone()).unwrap();
    assert_eq!(latencies.len(), 100);
    latencies.sort_by(f64::total_cmp);
    // The 50 ms p95 target holds on an idle machine. Under parallel loops the scheduler
    // latency alone exceeds it, so the default run only measures and prints. Assert it with:
    // `WS_ASSERT_LATENCY=1 cargo test -p ws --test tmux -- --nocapture`
    if common::latency_assertions_enabled() {
        assert!(!spool.exists() || fs::read_dir(&spool).unwrap().count() == 0);
        assert!(
            latencies[94] < 50.0,
            "Full ingest p95 exceeded 50 ms: {}",
            latencies[94]
        );
    }
    println!(
        "HOOK_ROUND_TRIP samples=100 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        latencies[49], latencies[94], latencies[99]
    );
}

#[test]
fn create_window_runs_a_single_executable_path_with_spaces() {
    let sandbox = Sandbox::new();
    let tmux = Tmux::named(&sandbox.server);
    let directory = sandbox.home.path().join("dir with spaces");
    fs::create_dir(&directory).unwrap();
    let marker = sandbox.home.path().join("marker");
    let executable = directory.join("touch marker");
    fs::write(
        &executable,
        format!("#!/bin/sh\n: > '{}'\n", marker.to_str().unwrap()),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

    let window = tmux
        .create_window("spike", "spaces", &[executable.to_str().unwrap()])
        .unwrap();
    assert!(window.starts_with('@'), "{window}");
    wait_until(|| marker.exists());
}

impl Sandbox {
    fn new() -> Self {
        let home = common::SandboxRoot::new();
        let sandbox = Self {
            server: home.tmux_socket(),
            home,
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
            .args(["-S", &self.server])
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

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "The sandbox operation timed out."
        );
        thread::sleep(Duration::from_millis(5));
    }
}
