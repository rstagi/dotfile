#[allow(dead_code)]
mod common;

use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};
use ws_core::{device::Device, ids::Ulid, log::SessionLog};
use ws_daemon::sessions::{CreateSession, Sessions};

#[test]
fn hook_and_usage_latency_on_a_large_registry() {
    let home = common::SandboxRoot::new();
    let state = home.path().join("state");
    let socket = home.path().join("wsd.sock");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let ids: Vec<_> = (0..4)
        .map(|_| sessions.create(CreateSession::default()).unwrap().id)
        .collect();
    drop(sessions);
    let device = Device::load(&state).unwrap();
    let padding = "x".repeat(16_384);
    for id in &ids {
        let mut writer = SessionLog::new(&state, *id, device.device_id).writer();
        writer
            .append_current(
                "runner.usage.updated",
                json!({
                    "ws.runner": "codex", "ws.runner.account.id": "generated-account",
                    "ws.runner.usage.window": "five_hour", "ws.runner.usage.source": "reported",
                    "ws.runner.usage.observed_at": 1_000, "ws.runner.usage.remaining_ratio": 0.75
                }),
            )
            .unwrap();
        for _ in 0..1_300 {
            writer
                .append_current("test.noise", json!({"padding": padding}))
                .unwrap();
        }
    }
    let mut daemon = common::Daemon::spawn(common::ws(home.path(), &socket));
    // Startup validates the full logs. Request timing starts after ping answers.
    daemon.wait_ready_for(&socket, Duration::from_secs(120));
    let bytes = fs::metadata(state.join("registry.sqlite")).unwrap().len();
    assert!(
        bytes >= 80_000_000,
        "The generated registry is too small: {bytes}."
    );
    let mut hook = Vec::new();
    let mut usage = Vec::new();
    for n in 0..100 {
        let callback = json!({
            "callback_id": Ulid::new(), "session_id": ids[n % ids.len()],
            "observed_at_us": 1_000_000 + n, "state": "working", "pane": null,
            "tmux_socket": null, "tmux_path": null, "payload": {}
        });
        let started = Instant::now();
        let response = common::request(&socket, "POST", "/v1/hooks/ingest", &callback.to_string());
        hook.push(started.elapsed());
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        let started = Instant::now();
        let response = common::get(&socket, "/v1/usage");
        usage.push(started.elapsed());
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(
            body["windows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["remaining_ratio"] == 0.75)
        );
    }
    hook.sort();
    usage.sort();
    let p95 = 94;
    eprintln!(
        "registry_bytes={bytes} samples=100 hook_p95={:?} usage_p95={:?} hook_max={:?} usage_max={:?}",
        hook[p95],
        usage[p95],
        hook.last().unwrap(),
        usage.last().unwrap()
    );
    // Measure CPU time across an idle interval. The interval is a measurement, not a readiness wait.
    let cpu_time = || {
        String::from_utf8(
            Command::new("ps")
                .args(["-p", &daemon.child.id().to_string(), "-o", "time="])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };
    let before = cpu_time();
    std::thread::sleep(Duration::from_secs(3));
    eprintln!(
        "idle_cpu_time_before={} idle_cpu_time_after={} interval_secs=3",
        before.trim(),
        cpu_time().trim()
    );
    if let Ok(destination) = std::env::var("WS_PERF_ARTIFACT_DIR") {
        let destination = std::path::Path::new(&destination);
        fs::create_dir_all(destination).unwrap();
        fs::copy(
            state.join("registry.sqlite"),
            destination.join("registry.sqlite"),
        )
        .unwrap();
    }
    if common::latency_assertions_enabled() {
        // Allow scheduler noise above the production targets of 50 ms and 200 ms.
        assert!(
            hook[p95] <= Duration::from_millis(75),
            "Hook ingest exceeded its latency margin: {:?}.",
            hook[p95]
        );
        assert!(
            usage[p95] <= Duration::from_millis(300),
            "Usage exceeded its latency margin: {:?}.",
            usage[p95]
        );
    }
}
