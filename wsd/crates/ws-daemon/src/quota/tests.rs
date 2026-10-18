use super::*;
use serde_json::{Value, json};
use ws_core::semconv as sc;

#[test]
fn s3_weekly_only_readings_preserve_unknown_windows_and_provenance() {
    let fixture = sample();
    let rpc = &fixture["app_server_responses"][1]["response"]["result"];
    let readings = codex::parse_app_server(rpc, "<redacted-account>", 1_792_186_317_783).unwrap();
    assert_eq!(readings.len(), 2);
    let weekly = readings.iter().find(|r| r.window == "weekly").unwrap();
    assert_eq!(weekly.duration_secs, Some(604800));
    assert_eq!(weekly.reset_ms, Some(1_792_653_579_000));
    assert_eq!(weekly.bucket.as_deref(), Some("codex"));
    assert_eq!(weekly.used_ratio, Some(0.45));
    assert_eq!(weekly.remaining_ratio(), Some(0.55));
    assert_eq!(weekly.source, Source::Reported);
    let missing = readings.iter().find(|r| r.window == "five_hour").unwrap();
    assert_eq!(missing.used_ratio, None);
    assert_eq!(missing.remaining_ratio(), None);
    let body = weekly.event_body();
    assert_eq!(body[sc::RUNNER_USAGE_OBSERVED_AT], 1_792_186_317_783_u64);
    assert_eq!(body[sc::RUNNER_ACCOUNT_ID], "<redacted-account>");
    assert!(missing.event_body()[sc::RUNNER_USAGE_USED_RATIO].is_null());

    let event = &fixture["rollout_events"][0];
    let rollout = codex::parse_rollout(event, "launch-account", 1_792_200_000_000).unwrap();
    let weekly = rollout.iter().find(|r| r.window == "weekly").unwrap();
    assert_eq!(weekly.account_id, "launch-account");
    assert_eq!(weekly.observed_at_ms, 1_792_186_353_638);
    assert_eq!(weekly.received_at_ms, 1_792_200_000_000);
    assert_eq!(weekly.source_name, "rollout");
    assert_eq!(weekly.used_ratio, Some(0.45));
    assert_eq!(weekly.reset_ms, Some(1_792_653_579_000));
}

#[test]
fn durations_buckets_and_expired_values_are_independent_of_window_position() {
    let response = json!({"accountId": "a", "rateLimitsByLimitId": {
        "codex": {"primary": {"windowDurationMins": 10080, "usedPercent": 20, "resetsAt": 5000},
                   "secondary": {"windowDurationMins": 300, "usedPercent": 0, "resetsAt": 4000}},
        "review": {"primary": {"windowDurationMins": 60, "usedPercent": 80, "resetsAt": 1000}}
    }});
    let readings = codex::parse_app_server(&response, "a", 1_000_000).unwrap();
    assert_eq!(readings.len(), 5);
    let five = readings
        .iter()
        .find(|r| r.window == "five_hour" && r.bucket.as_deref() == Some("codex"))
        .unwrap();
    assert_eq!(five.used_ratio, Some(0.0));
    assert_eq!(five.remaining_ratio(), Some(1.0));
    let other = readings
        .iter()
        .find(|r| r.window == "duration:3600")
        .unwrap();
    assert_eq!(other.bucket.as_deref(), Some("review"));
    assert_eq!(other.duration_secs, Some(3600));
    assert_eq!(
        other.used_ratio, None,
        "An expired snapshot must not imply current capacity."
    );
    assert!(codex::parse_app_server(&response, "b", 0).is_err());
    let invalid = json!({"accountId":"a", "rateLimits": {"primary": {"windowDurationMins": 300, "usedPercent": 101}}});
    assert!(
        codex::parse_app_server(&invalid, "a", 0)
            .unwrap()
            .iter()
            .all(|r| r.used_ratio.is_none())
    );
}

#[test]
fn shared_preparation_captures_the_launch_account_without_credentials() {
    use crate::agents::{AgentSession, Mode, prepare};
    let sandbox = tempfile::tempdir().unwrap();
    let codex_home = sandbox.path().join("custom-codex");
    std::fs::create_dir(&codex_home).unwrap();
    let auth = codex_home.join("auth.json");
    std::fs::write(
        &auth,
        r#"{"tokens":{"account_id":"account-a","access_token":"secret"}}"#,
    )
    .unwrap();
    let mut context = AgentSession {
        agent_program: Some("/fake/codex".into()),
        state: sandbox.path().join("state"),
        parent_session_id: None,
        project: "sandbox".into(),
        id: ws_core::ids::Ulid::new().to_string(),
        home: sandbox.path().into(),
        codex_home: Some(codex_home.clone()),
        grok_home: None,
        ws_home: sandbox.path().join("ws"),
        ws_bin: "/fake/ws".into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    let prepared = prepare("codex", Mode::Headless, &context, Some("conversation-a")).unwrap();
    let binding_file = prepared
        .files
        .iter()
        .find(|file| file.path.ends_with("quota-binding.json"))
        .unwrap();
    let original = binding_file.contents.clone();
    let binding: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(binding["account_id"], "account-a");
    assert_eq!(binding["codex_home"], codex_home.to_str().unwrap());
    assert_eq!(binding["resume_id"], "conversation-a");
    assert!(!String::from_utf8_lossy(&original).contains("secret"));
    prepared.write_files().unwrap();
    std::fs::write(&auth, r#"{"tokens":{"account_id":"account-b"}}"#).unwrap();
    let resumed = prepare("codex", Mode::Interactive, &context, None).unwrap();
    let binding_file = resumed
        .files
        .iter()
        .find(|file| file.path.ends_with("quota-binding.json"))
        .unwrap();
    assert_eq!(binding_file.contents, original);
    context.id = ws_core::ids::Ulid::new().to_string();
    let next = prepare("codex", Mode::Headless, &context, None).unwrap();
    let next_binding = next
        .files
        .iter()
        .find(|file| file.path.ends_with("quota-binding.json"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&next_binding.contents).unwrap()["account_id"],
        "account-b"
    );
}

#[tokio::test]
async fn app_server_initializes_then_reads_quota_without_starting_inference() {
    use std::{fs, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let program = fake_app_server(sandbox.path());
    let binding = binding::Binding {
        runner: "codex".into(),
        account_id: Some("<redacted-account>".into()),
        codex_home: sandbox.path().into(),
        program,
        resume_id: None,
    };
    let mut reader =
        codex::CodexReader::new(binding, sandbox.path().into(), ws_core::ids::Ulid::new());
    let readings = reader.read(Arc::new(|| 1_792_186_317_783)).await.unwrap();
    assert_eq!(
        readings
            .iter()
            .find(|r| r.window == "weekly")
            .unwrap()
            .used_ratio,
        Some(0.45)
    );
    assert_eq!(readings[0].source_name, "app-server");
    assert_eq!(readings[0].received_at_ms, 1_792_186_317_783);
    let methods: Vec<_> = fs::read_to_string(sandbox.path().join("requests"))
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        methods,
        ["initialize", "initialized", "account/rateLimits/read"]
    );
}

#[tokio::test]
async fn passive_rollouts_keep_the_launch_account_and_original_age() {
    use std::{fs, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let directory = sandbox.path().join("sessions/2026/10/16");
    fs::create_dir_all(&directory).unwrap();
    let event = sample()["rollout_events"][0].clone();
    fs::write(
        directory.join("rollout-conversation-a.jsonl"),
        format!("{event}\n{{partial"),
    )
    .unwrap();
    let mut unrelated = event.clone();
    unrelated["payload"]["rate_limits"]["primary"]["used_percent"] = json!(99);
    fs::write(
        directory.join("rollout-conversation-b.jsonl"),
        format!("{unrelated}\n"),
    )
    .unwrap();
    fs::write(
        sandbox.path().join("auth.json"),
        r#"{"tokens":{"account_id":"new-account"}}"#,
    )
    .unwrap();
    let binding = binding::Binding {
        runner: "codex".into(),
        account_id: Some("launch-account".into()),
        codex_home: sandbox.path().into(),
        program: sandbox.path().join("absent"),
        resume_id: Some("conversation-a".into()),
    };
    let mut reader = codex::CodexReader::new(
        binding,
        sandbox.path().join("state"),
        ws_core::ids::Ulid::new(),
    );
    let readings = reader.read(Arc::new(|| 1_792_200_000_000)).await.unwrap();
    let weekly = readings.iter().find(|r| r.window == "weekly").unwrap();
    assert_eq!(weekly.account_id, "launch-account");
    assert_eq!(weekly.used_ratio, Some(0.45));
    assert_eq!(weekly.observed_at_ms, 1_792_186_353_638);
    assert_eq!(weekly.received_at_ms, 1_792_200_000_000);
    assert_eq!(weekly.source_name, "rollout");
}

#[tokio::test(start_paused = true)]
async fn timer_ticks_and_each_phase_boundary_write_durable_quota_events() {
    use crate::sessions::{CreateSession, Sessions};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };
    struct SampleReader;
    impl Reader for SampleReader {
        fn read(&mut self, clock: Clock) -> ReadFuture<'_> {
            Box::pin(async move {
                codex::parse_app_server(
                    &sample()["app_server_responses"][1]["response"]["result"],
                    "<redacted-account>",
                    clock(),
                )
            })
        }
    }
    let sandbox = tempfile::tempdir().unwrap();
    let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    let mut changes = sessions.changes.subscribe();
    let log = ws_core::log::SessionLog::new(sandbox.path(), session.id, session.device_id);
    let store = Arc::new(Mutex::new(sessions));
    let time = Arc::new(AtomicU64::new(1_792_186_317_783));
    let clock: Clock = Arc::new(move || time.fetch_add(1, Ordering::SeqCst));
    let mut collector = Collector::new(clock);
    collector.register(session.id, Box::new(SampleReader));
    let service = Service::start_with_collector(store.clone(), collector).unwrap();
    wait_for_usage(&log, &mut changes, 2).await;
    tokio::time::advance(std::time::Duration::from_secs(59)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        log.read()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.event_type == "runner.usage.updated")
            .count(),
        2
    );
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    wait_for_usage(&log, &mut changes, 4).await;
    {
        let mut sessions = store.lock().unwrap();
        sessions
            .append(session.id, "phase.started", json!({"phase":"46"}))
            .unwrap();
        sessions
            .append(session.id, "phase.finished", json!({"phase":"46"}))
            .unwrap();
    }
    wait_for_usage(&log, &mut changes, 8).await;
    store.lock().unwrap().shutdown.send_replace(true);
    service.shutdown().await;
    let events: Vec<_> = log
        .read()
        .unwrap()
        .entries
        .into_iter()
        .filter(|e| e.event_type == "runner.usage.updated")
        .collect();
    assert_eq!(events.len(), 8);
    assert!(
        events
            .iter()
            .all(|entry| entry.body[sc::RUNNER_ACCOUNT_ID] == "<redacted-account>")
    );
    assert_eq!(events[0].body[sc::RUNNER_USAGE_SOURCE], "reported");
    let device = ws_core::device::Device::load(sandbox.path()).unwrap();
    let telemetry = ws_core::telemetry_log::TelemetryLog::new(sandbox.path(), device.device_id)
        .read()
        .unwrap();
    let records: Vec<_> = telemetry
        .iter()
        .flat_map(|entry| entry.payload["resourceLogs"].as_array().unwrap())
        .flat_map(|resource| resource["scopeLogs"].as_array().unwrap())
        .flat_map(|scope| scope["logRecords"].as_array().unwrap())
        .filter(|record| record["eventName"] == sc::RUNNER_USAGE_UPDATED)
        .collect();
    assert_eq!(records.len(), 8);
    assert_eq!(records[0]["timeUnixNano"], "1792186317783000000");
    assert!(
        records[0]["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|attribute| attribute["key"] == sc::RUNNER_ACCOUNT_ID
                && attribute["value"]["stringValue"] == "<redacted-account>")
    );
    drop(store);
    let restored = Sessions::open(sandbox.path(), 1).unwrap();
    drop(restored);
    assert_eq!(
        log.read()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.event_type == "runner.usage.updated")
            .count(),
        8
    );
}

async fn wait_for_usage(
    log: &ws_core::log::SessionLog,
    changes: &mut tokio::sync::watch::Receiver<()>,
    count: usize,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if log
            .read()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.event_type == "runner.usage.updated")
            .count()
            >= count
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "The collector did not write the quota events."
        );
        tokio::select! {
            biased;
            _ = changes.changed() => {},
            _ = tokio::task::yield_now() => {},
        }
    }
}

#[test]
fn rpc_identity_is_required_and_an_unreported_payload_keeps_windows_unknown() {
    let missing_account =
        json!({"rateLimits": {"primary": {"usedPercent": 10,"windowDurationMins":300}}});
    assert!(codex::parse_app_server(&missing_account, "launch-account", 0).is_err());
    let unknown =
        codex::parse_app_server(&json!({"accountId":"launch-account"}), "launch-account", 50)
            .unwrap();
    assert_eq!(unknown.len(), 2);
    assert!(
        unknown
            .iter()
            .all(|reading| reading.used_ratio.is_none() && reading.reset_ms.is_none())
    );
    assert!(unknown.iter().all(|reading| {
        reading
            .event_body()
            .get(sc::RUNNER_USAGE_USED_RATIO)
            .is_none()
    }));
}

#[tokio::test]
async fn an_unreadable_rollout_source_does_not_disable_the_app_server() {
    use std::{fs, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let program = fake_app_server(sandbox.path());
    fs::write(
        sandbox.path().join("sessions"),
        "This path is not a directory.",
    )
    .unwrap();
    let binding = binding::Binding {
        runner: "codex".into(),
        account_id: Some("<redacted-account>".into()),
        codex_home: sandbox.path().into(),
        program,
        resume_id: Some("conversation-a".into()),
    };
    let mut reader = codex::CodexReader::new(
        binding,
        sandbox.path().join("state"),
        ws_core::ids::Ulid::new(),
    );
    let readings = reader.read(Arc::new(|| 1_792_186_317_783)).await.unwrap();
    assert_eq!(
        readings
            .iter()
            .find(|r| r.window == "weekly")
            .unwrap()
            .used_ratio,
        Some(0.45)
    );
    assert!(readings.iter().all(|r| r.source_name == "app-server"));
}

#[tokio::test]
async fn daemon_collection_discovers_the_shared_launch_binding() {
    use crate::{
        agents::{AgentSession, Mode, prepare},
        sessions::{CreateSession, Sessions},
    };
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    let sandbox = tempfile::tempdir().unwrap();
    let program = fake_app_server(sandbox.path());
    fs::write(
        sandbox.path().join("auth.json"),
        r#"{"tokens":{"account_id":"<redacted-account>"}}"#,
    )
    .unwrap();
    let state = sandbox.path().join("state");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    let context = AgentSession {
        agent_program: Some(program),
        state: state.clone(),
        parent_session_id: None,
        project: "sandbox".into(),
        id: session.id.to_string(),
        home: sandbox.path().into(),
        codex_home: Some(sandbox.path().into()),
        grok_home: None,
        ws_home: sandbox.path().join("ws"),
        ws_bin: "/fake/ws".into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    prepare("codex", Mode::Headless, &context, None)
        .unwrap()
        .write_files()
        .unwrap();
    let log = ws_core::log::SessionLog::new(&state, session.id, session.device_id);
    let mut changes = sessions.changes.subscribe();
    let store = Arc::new(Mutex::new(sessions));
    let service = Service::start(store.clone()).unwrap();
    wait_for_usage(&log, &mut changes, 2).await;
    store.lock().unwrap().shutdown.send_replace(true);
    service.shutdown().await;
    let events: Vec<_> = log
        .read()
        .unwrap()
        .entries
        .into_iter()
        .filter(|e| e.event_type == "runner.usage.updated")
        .collect();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|entry| entry.body[sc::RUNNER_ACCOUNT_ID] == "<redacted-account>")
    );
    assert!(
        events
            .iter()
            .any(|entry| entry.body[sc::RUNNER_USAGE_WINDOW] == "five_hour"
                && entry.body.get(sc::RUNNER_USAGE_USED_RATIO).is_none())
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_app_server_is_killed_and_reaped_at_the_injected_deadline() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::Arc,
        time::{Duration, Instant},
    };
    let sandbox = tempfile::tempdir().unwrap();
    let program = sandbox.path().join("codex");
    fs::write(&program, "#!/bin/sh\nIFS= read -r request\nprintf '%s' \"$$\" > \"$CODEX_HOME/ready\"\nexec /bin/sleep 3600\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let binding = binding::Binding {
        runner: "codex".into(),
        account_id: Some("a".into()),
        codex_home: sandbox.path().into(),
        program,
        resume_id: None,
    };
    let mut reader = codex::CodexReader::new(
        binding,
        sandbox.path().join("state"),
        ws_core::ids::Ulid::new(),
    );
    let task = tokio::spawn(async move { reader.read(Arc::new(|| 100)).await });
    let deadline = Instant::now() + Duration::from_secs(30);
    let pid = loop {
        if let Some(pid) = fs::read_to_string(sandbox.path().join("ready"))
            .ok()
            .and_then(|text| text.parse::<i32>().ok())
        {
            break pid;
        }
        assert!(
            Instant::now() < deadline,
            "The fake app-server did not signal readiness."
        );
        tokio::task::yield_now().await;
    };
    tokio::time::advance(Duration::from_secs(30)).await;
    assert_eq!(
        task.await.unwrap().unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    // SAFETY: Signal zero only checks the process that this test started.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

fn sample() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/codex.json")).unwrap()
}

fn fake_app_server(directory: &std::path::Path) -> std::path::PathBuf {
    use std::{fs, os::unix::fs::PermissionsExt};
    let program = directory.join("codex");
    fs::write(
        &program,
        r#"#!/bin/sh
[ "$1" = app-server ] || exit 31
IFS= read -r line
printf '%s\n' "$line" >> "$CODEX_HOME/requests"
case "$line" in *'"method":"initialize"'*) ;; *) exit 32 ;; esac
printf '{"method":"server/ready","params":{}}\n{"id":1,"result":{}}\n'
IFS= read -r line
printf '%s\n' "$line" >> "$CODEX_HOME/requests"
case "$line" in *'"method":"initialized"'*) ;; *) exit 33 ;; esac
IFS= read -r line
printf '%s\n' "$line" >> "$CODEX_HOME/requests"
case "$line" in *'"method":"account/rateLimits/read"'*) ;; *) exit 34 ;; esac
cat "$CODEX_HOME/response.json"
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let mut response = sample()["app_server_responses"][1]["response"].clone();
    response["id"] = json!(2);
    fs::write(directory.join("response.json"), format!("{response}\n")).unwrap();
    program
}
