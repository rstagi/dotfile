mod common;

use serde_json::{Value, json};
use ws_core::{device::Device, log::SessionLog};
use ws_daemon::sessions::{CreateSession, Sessions};

#[test]
fn usage_shows_all_windows_with_provenance_age_and_explicit_unknown_values() {
    let home = common::SandboxRoot::new();
    let state = home.path().join("state");
    let socket = home.path().join("wsd.sock");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    drop(sessions);
    let device = Device::load(&state).unwrap();
    let log = SessionLog::new(&state, session.id, device.device_id);
    for (window, remaining, source) in [
        ("five_hour", Some(0.75), "reported"),
        ("weekly", Some(0.25), "estimated"),
        ("other", None, "estimated"),
    ] {
        log.append(
            1,
            "runner.usage.updated",
            json!({
                "ws.runner": "codex", "ws.runner.account.id": "account-a",
                "ws.runner.usage.window": window,
                "ws.runner.usage.remaining_ratio": remaining,
                "ws.runner.usage.source": source,
                "ws.runner.usage.observed_at": 1_000,
                "received_at_ms": 9_000
            }),
        )
        .unwrap();
    }
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/usage").starts_with("HTTP/1.1 200"));
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let output = common::ws(home.path(), &socket)
        .args(["usage", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    let windows = data["windows"].as_array().unwrap();
    let codex: Vec<_> = windows.iter().filter(|w| w["runner"] == "codex").collect();
    assert_eq!(codex.len(), 3);
    let five_hour = codex.iter().find(|w| w["window"] == "five_hour").unwrap();
    assert_eq!(five_hour["remaining_ratio"], 0.75);
    assert_eq!(five_hour["source"], "reported");
    assert!(five_hour["age_secs"].as_u64().unwrap() >= before - 1);
    let weekly = codex.iter().find(|w| w["window"] == "weekly").unwrap();
    assert_eq!(weekly["remaining_ratio"], 0.25);
    assert_eq!(weekly["source"], "estimated");
    let unknown = codex.iter().find(|w| w["window"] == "other").unwrap();
    assert!(unknown["remaining_ratio"].is_null());
    assert_eq!(unknown["status"], "unknown");
    for runner in ["claude", "grok"] {
        let unavailable = windows.iter().find(|w| w["runner"] == runner).unwrap();
        assert_eq!(unavailable["status"], "unavailable");
        assert!(unavailable["remaining_ratio"].is_null());
        assert!(unavailable["age_secs"].is_null());
    }
    let output = common::ws(home.path(), &socket)
        .arg("usage")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("75.0%"), "{text}");
    assert!(text.contains("reported"), "{text}");
    assert!(text.contains("estimated"), "{text}");
    assert!(text.contains("unknown"), "{text}");
    assert!(text.contains("unavailable"), "{text}");
    assert!(text.contains("ago"), "{text}");
}

#[test]
fn usage_keeps_accounts_and_buckets_separate_and_uses_observation_time() {
    let home = common::SandboxRoot::new();
    let state = home.path().join("state");
    let socket = home.path().join("wsd.sock");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let a = sessions.create(CreateSession::default()).unwrap();
    let b = sessions.create(CreateSession::default()).unwrap();
    drop(sessions);
    let device = Device::load(&state).unwrap();
    let first = SessionLog::new(&state, a.id, device.device_id);
    let second = SessionLog::new(&state, b.id, device.device_id);
    let reading = |account: &str, bucket: &str, at: u64, ratio: f64| {
        json!({
            "ws.runner": "codex", "ws.runner.account.id": account,
            "ws.runner.usage.bucket": bucket, "ws.runner.usage.window": "five_hour",
            "ws.runner.usage.source": "reported", "ws.runner.usage.observed_at": at,
            "ws.runner.usage.remaining_ratio": ratio, "received_at_ms": now,
        })
    };
    first
        .append(
            1,
            "runner.usage.updated",
            reading("a", "standard", now - 2_000, 0.25),
        )
        .unwrap();
    second
        .append(
            1,
            "runner.usage.updated",
            reading("a", "standard", now - 1_000, 0.5),
        )
        .unwrap();
    first
        .append(
            1,
            "runner.usage.updated",
            reading("a", "standard", now - 4_000, 0.9),
        )
        .unwrap();
    first
        .append(
            1,
            "runner.usage.updated",
            reading("a", "premium", now - 1_000, 0.1),
        )
        .unwrap();
    first
        .append(
            1,
            "runner.usage.updated",
            reading("b", "standard", now - 1_000, 0.2),
        )
        .unwrap();
    for (window, ratio, extra) in [
        (
            "expired",
            1.0,
            json!({"ws.runner.usage.window.reset": now - 1}),
        ),
        ("invalid", 2.0, json!({})),
        (
            "future",
            0.0,
            json!({"ws.runner.usage.observed_at": now + 3_600_000}),
        ),
    ] {
        let mut body = reading("a", "standard", now - 1_000, ratio);
        body["ws.runner.usage.window"] = window.into();
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        first.append(1, "runner.usage.updated", body).unwrap();
    }
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let output = common::ws(home.path(), &socket)
        .args(["usage", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    let windows = data["windows"].as_array().unwrap();
    let find = |account: &str, bucket: &str, window: &str| {
        windows
            .iter()
            .find(|w| w["account_id"] == account && w["bucket"] == bucket && w["window"] == window)
            .unwrap()
    };
    assert_eq!(find("a", "standard", "five_hour")["remaining_ratio"], 0.5);
    assert_eq!(find("a", "premium", "five_hour")["remaining_ratio"], 0.1);
    assert_eq!(find("b", "standard", "five_hour")["remaining_ratio"], 0.2);
    for (window, status) in [
        ("expired", "unavailable"),
        ("invalid", "unknown"),
        ("future", "unknown"),
    ] {
        let reading = find("a", "standard", window);
        assert!(reading["remaining_ratio"].is_null());
        assert_eq!(reading["status"], status);
    }
    assert!(find("a", "standard", "future")["age_secs"].is_null());
}

#[test]
fn usage_labels_the_logged_in_account_and_hides_windows_the_plan_does_not_report() {
    let home = common::SandboxRoot::new();
    let state = home.path().join("state");
    let socket = home.path().join("wsd.sock");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // A fake unsigned ID token with only an email claim: {"email":"user@example.com"}.
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    std::fs::write(
        home.path().join(".codex/auth.json"),
        json!({"tokens": {"account_id": "account-a", "access_token": "fake-access",
            "id_token": "e30.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20ifQ.fake-signature"}})
        .to_string(),
    )
    .unwrap();
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    drop(sessions);
    let device = Device::load(&state).unwrap();
    let log = SessionLog::new(&state, session.id, device.device_id);
    for (bucket, window, remaining, source, at) in [
        (None, "five_hour", None, "estimated", now - 3_000),
        (None, "weekly", None, "estimated", now - 3_000),
        (Some("codex"), "five_hour", None, "estimated", now - 2_000),
        (Some("codex"), "weekly", Some(0.92), "reported", now - 1_000),
    ] {
        log.append(
            1,
            "runner.usage.updated",
            json!({
                "ws.runner": "codex", "ws.runner.account.id": "account-a",
                "ws.runner.usage.bucket": bucket, "ws.runner.usage.window": window,
                "ws.runner.usage.remaining_ratio": remaining,
                "ws.runner.usage.source": source,
                "ws.runner.usage.observed_at": at, "received_at_ms": at,
            }),
        )
        .unwrap();
    }
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let output = common::ws(home.path(), &socket)
        .args(["usage", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("fake"), "The output holds no token.");
    let data: Value = serde_json::from_str(&text).unwrap();
    let codex: Vec<_> = data["windows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["runner"] == "codex")
        .collect();
    assert_eq!(codex.len(), 1, "{codex:?}");
    assert_eq!(codex[0]["window"], "weekly");
    assert_eq!(codex[0]["bucket"], "codex");
    assert_eq!(codex[0]["remaining_ratio"], 0.92);
    assert_eq!(codex[0]["account_id"], "account-a");
    assert_eq!(codex[0]["account_label"], "user@example.com");
}
