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
        claude_config_dir: None,
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
        claude_config_dir: None,
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

#[test]
fn claude_s3_sources_normalize_percentages_and_fractions_to_ratios() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/claude.json")).unwrap();
    let interactive = claude::parse_status_line(
        &fixture["statusline_callbacks"][1]["input"],
        "launch-account",
        1_792_186_352_000,
    );
    let headless = claude::parse_stream_event(
        &fixture["headless"]["stream_events"][0],
        "launch-account",
        1_792_186_416_000,
    );
    for readings in [&interactive, &headless] {
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].window, "five_hour");
        assert_eq!(readings[0].used_ratio, Some(0.0));
        assert_eq!(readings[0].reset_ms, Some(1_792_203_600_000));
        assert_eq!(readings[1].window, "weekly");
        assert_eq!(readings[1].duration_secs, Some(604800));
        assert_eq!(readings[1].used_ratio, Some(0.14));
        assert_eq!(readings[1].remaining_ratio(), Some(0.86));
        assert_eq!(readings[1].account_id, "launch-account");
        assert_eq!(readings[1].source, Source::Reported);
        assert_eq!(readings[1].reset_ms, Some(1_792_483_200_000));
    }
    assert_eq!(interactive[1].source_name, "status-line");
    assert_eq!(headless[1].source_name, "stream-json");
}

#[test]
fn claude_preparation_forwards_status_input_and_keeps_the_launch_account() {
    use crate::agents::{AgentSession, Mode, prepare};
    use std::fs;
    let sandbox = tempfile::tempdir().unwrap();
    let config = sandbox.path().join(".claude.json");
    fs::write(&config, r#"{"oauthAccount":{"accountUuid":"account-a","emailAddress":"private"},"secret":"credential"}"#).unwrap();
    let context = AgentSession {
        claude_config_dir: None,
        agent_program: Some("/fake/claude".into()),
        state: sandbox.path().join("state"),
        parent_session_id: None,
        project: "sandbox".into(),
        id: ws_core::ids::Ulid::new().to_string(),
        home: sandbox.path().into(),
        codex_home: None,
        grok_home: None,
        ws_home: sandbox.path().join("ws"),
        ws_bin: sandbox.path().join("ws with spaces"),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    for mode in [Mode::Interactive, Mode::Headless] {
        let prepared = prepare("claude", mode, &context, Some("conversation-a")).unwrap();
        let settings = prepared
            .files
            .iter()
            .find(|f| f.path.ends_with("claude-settings.json"))
            .unwrap();
        let settings: Value = serde_json::from_slice(&settings.contents).unwrap();
        assert_eq!(settings["statusLine"]["type"], "command");
        let command = settings["statusLine"]["command"].as_str().unwrap();
        assert_eq!(
            shell_words::split(command).unwrap(),
            [context.ws_bin.to_str().unwrap(), "hook", "quota"]
        );
        let binding = prepared
            .files
            .iter()
            .find(|f| f.path.ends_with("quota-binding.json"))
            .unwrap();
        let original = binding.contents.clone();
        let value: Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(value["runner"], "claude");
        assert_eq!(value["account_id"], "account-a");
        assert!(!String::from_utf8_lossy(&original).contains("private"));
        assert!(!String::from_utf8_lossy(&original).contains("credential"));
        prepared.write_files().unwrap();
        fs::write(&config, r#"{"oauthAccount":{"accountUuid":"account-b"}}"#).unwrap();
        let resumed = prepare("claude", mode, &context, None).unwrap();
        assert_eq!(
            resumed
                .files
                .iter()
                .find(|f| f.path.ends_with("quota-binding.json"))
                .unwrap()
                .contents,
            original
        );
    }
}

#[tokio::test]
async fn claude_status_callbacks_become_durable_usage_without_changing_session_state() {
    use crate::{
        agents::{AgentSession, Mode, prepare},
        hooks::{self, Callback, StopDecision},
        sessions::{CreateSession, Sessions},
    };
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    let sandbox = tempfile::tempdir().unwrap();
    fs::write(
        sandbox.path().join(".claude.json"),
        r#"{"oauthAccount":{"accountUuid":"launch-account"}}"#,
    )
    .unwrap();
    let state = sandbox.path().join("state");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    let context = AgentSession {
        claude_config_dir: None,
        agent_program: Some("/fake/claude".into()),
        state: state.clone(),
        parent_session_id: None,
        project: "sandbox".into(),
        id: session.id.to_string(),
        home: sandbox.path().into(),
        codex_home: None,
        grok_home: None,
        ws_home: sandbox.path().join("ws"),
        ws_bin: "/fake/ws".into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    prepare("claude", Mode::Interactive, &context, None)
        .unwrap()
        .write_files()
        .unwrap();
    let mut changes = sessions.changes.subscribe();
    let log = ws_core::log::SessionLog::new(&state, session.id, session.device_id);
    let store = Arc::new(Mutex::new(sessions));
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/claude.json")).unwrap();
    hooks::ingest(
        store.clone(),
        Callback {
            callback_id: ws_core::ids::Ulid::new(),
            session_id: session.id,
            observed_at_us: 1_792_186_352_000_000,
            state: "quota".into(),
            pane: None,
            tmux_socket: None,
            tmux_path: None,
            payload: fixture["statusline_callbacks"][1]["input"].clone(),
            notify: None,
        },
        StopDecision::Accept,
    )
    .await
    .unwrap();
    let collector = Collector::new(Arc::new(|| 1_792_186_360_000));
    let service = Service::start_with_collector(store.clone(), collector).unwrap();
    wait_for_usage(&log, &mut changes, 2).await;
    store.lock().unwrap().shutdown.send_replace(true);
    service.shutdown().await;
    let entries = log.read().unwrap().entries;
    let readings: Vec<_> = entries
        .iter()
        .filter(|e| e.event_type == "runner.usage.updated")
        .collect();
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[1].body[sc::RUNNER_USAGE_USED_RATIO], 0.14);
    assert_eq!(readings[1].body[sc::RUNNER_ACCOUNT_ID], "launch-account");
    assert_eq!(
        readings[1].body[sc::RUNNER_USAGE_OBSERVED_AT],
        1_792_186_352_000_u64
    );
    assert_eq!(
        store
            .lock()
            .unwrap()
            .get(&session.id.to_string(), None)
            .unwrap()
            .state,
        session.state
    );
    assert!(
        !entries
            .iter()
            .any(|e| ["turn.started", "turn.ended"].contains(&e.event_type.as_str()))
    );
}

#[tokio::test]
async fn claude_stream_reader_preserves_unknown_windows_and_the_first_receipt_time() {
    use std::{fs, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let id = ws_core::ids::Ulid::new();
    let output = sandbox.path().join("stdout.log");
    let log = ws_core::log::SessionLog::new(sandbox.path(), id, ws_core::ids::Ulid::nil());
    log.writer()
        .append_current(
            "session.created",
            json!({"kind":"headless", "stdout": output}),
        )
        .unwrap();
    fs::write(
        &output,
        "{\"type\":\"system\"}\n{\"type\":\"result\",\"usage\":{\"output_tokens\":236}}\n",
    )
    .unwrap();
    let mut reader = claude::ClaudeReader::new(
        claude::Binding {
            runner: "claude".into(),
            account_id: Some("launch-account".into()),
        },
        sandbox.path().into(),
        id,
    );
    let unknown = reader.read(Arc::new(|| 1_792_186_416_000)).await.unwrap();
    assert_eq!(unknown.len(), 2);
    assert!(
        unknown
            .iter()
            .all(|r| r.used_ratio.is_none() && r.tokens.is_none())
    );
    assert_eq!(unknown[0].source_name, "stream-json");
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/claude.json")).unwrap();
    let event = &fixture["headless"]["stream_events"][0];
    fs::write(
        &output,
        format!("{{\"type\":\"system\"}}\n{event}\n{{partial"),
    )
    .unwrap();
    let known = reader.read(Arc::new(|| 1_792_186_420_000)).await.unwrap();
    assert_eq!(known[1].used_ratio, Some(0.14));
    let repeated = reader.read(Arc::new(|| 1_792_186_480_000)).await.unwrap();
    assert_eq!(repeated[1].observed_at_ms, 1_792_186_420_000);
    assert_eq!(repeated[1].received_at_ms, 1_792_186_480_000);
    let expired = reader.read(Arc::new(|| 1_792_483_200_000)).await.unwrap();
    assert!(expired.iter().all(|r| r.used_ratio.is_none()));
}

#[test]
fn claude_missing_malformed_and_expired_fields_never_imply_capacity() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/claude.json")).unwrap();
    let missing =
        claude::parse_status_line(&fixture["statusline_callbacks"][0]["input"], "account", 50);
    assert_eq!(missing.len(), 2);
    assert!(
        missing
            .iter()
            .all(|r| r.used_ratio.is_none() && r.reset_ms.is_none())
    );
    for value in [
        json!(null),
        json!("0"),
        json!(-1),
        json!(101),
        json!({}),
        json!([]),
    ] {
        let input = json!({"rate_limits":{"five_hour":{"used_percentage":value,"resets_at":100}}});
        let readings = claude::parse_status_line(&input, "account", 50);
        assert!(
            readings
                .iter()
                .all(|r| r.event_body().get(sc::RUNNER_USAGE_USED_RATIO).is_none())
        );
        assert_eq!(readings[0].reset_ms, Some(100_000));
    }
    for value in [
        json!(null),
        json!("0.14"),
        json!(-0.1),
        json!(14),
        json!({}),
    ] {
        let input = json!({"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"seven_day":{"utilization":value}}}});
        assert!(
            claude::parse_stream_event(&input, "account", 50)
                .iter()
                .all(|r| r.used_ratio.is_none())
        );
    }
    let expired = json!({"rate_limits":{"five_hour":{"used_percentage":0,"resets_at":1},"seven_day":{"used_percentage":100,"resets_at":2}}});
    let readings = claude::parse_status_line(&expired, "account", 1_000);
    assert_eq!(readings[0].used_ratio, None);
    assert_eq!(readings[1].used_ratio, Some(1.0));
    assert_eq!(readings[1].remaining_ratio(), Some(0.0));
    assert!(claude::parse_status_line(&expired, "", 0).is_empty());
    assert!(
        claude::parse_stream_event(
            &json!({"type":"result","usage":{"output_tokens":99}}),
            "account",
            0
        )
        .is_empty()
    );
}

#[tokio::test]
async fn repeated_status_callbacks_retain_the_observation_time_across_reader_restarts() {
    use std::sync::Arc;
    let sandbox = tempfile::tempdir().unwrap();
    let id = ws_core::ids::Ulid::new();
    let log = ws_core::log::SessionLog::new(sandbox.path(), id, ws_core::ids::Ulid::nil());
    let input = json!({"rate_limits":{"five_hour":{"used_percentage":25,"resets_at":10000}}});
    for time in [1_000_000_u64, 2_000_000] {
        log.writer()
            .append_current(
                "hook.received",
                json!({"state":"quota","observed_at_us":time,"payload":input}),
            )
            .unwrap();
    }
    for _ in 0..2 {
        let mut reader = claude::ClaudeReader::new(
            claude::Binding {
                runner: "claude".into(),
                account_id: Some("account".into()),
            },
            sandbox.path().into(),
            id,
        );
        let readings = reader.read(Arc::new(|| 5000)).await.unwrap();
        assert_eq!(readings[0].used_ratio, Some(0.25));
        assert_eq!(readings[0].observed_at_ms, 1000);
        assert_eq!(readings[0].received_at_ms, 5000);
    }
}

#[tokio::test]
async fn delayed_status_callbacks_do_not_replace_a_newer_reading() {
    use std::sync::Arc;
    let sandbox = tempfile::tempdir().unwrap();
    let id = ws_core::ids::Ulid::new();
    let log = ws_core::log::SessionLog::new(sandbox.path(), id, ws_core::ids::Ulid::nil());
    for (time, used) in [(1_000_000_u64, 25), (3_000_000, 50), (2_000_000, 35)] {
        log.writer()
            .append_current(
                "hook.received",
                json!({"state":"quota","observed_at_us":time,
            "payload":{"rate_limits":{"five_hour":{"used_percentage":used,"resets_at":10000}}}}),
            )
            .unwrap();
    }
    let mut reader = claude::ClaudeReader::new(
        claude::Binding {
            runner: "claude".into(),
            account_id: Some("account".into()),
        },
        sandbox.path().into(),
        id,
    );
    let readings = reader.read(Arc::new(|| 5000)).await.unwrap();
    assert_eq!(readings[0].used_ratio, Some(0.5));
    assert_eq!(readings[0].observed_at_ms, 3000);
}

#[test]
fn grok_billing_preserves_the_weekly_period_and_missing_percentage() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/grok.json")).unwrap();
    let result = &fixture["acp_responses"][0]["response"]["result"];
    let now = 1_792_186_404_245;
    let readings = grok::parse_billing(result, "user-a", now);
    assert_eq!(readings.len(), 1);
    let reading = &readings[0];
    assert_eq!(reading.window, "weekly");
    assert_eq!(reading.duration_secs, Some(604800));
    assert_eq!(reading.start_ms, Some(1_791_972_561_510));
    assert_eq!(reading.reset_ms, Some(1_792_577_361_510));
    assert_eq!(reading.remaining_ratio(), None);
    assert_eq!(reading.source, Source::Reported);
    assert_eq!(reading.observed_at_ms, now);
    assert!(
        reading
            .event_body()
            .get(sc::RUNNER_USAGE_USED_RATIO)
            .is_none()
    );
    let mut reported = result.clone();
    reported["config"]["creditUsagePercent"] = json!(25);
    assert_eq!(
        grok::parse_billing(&reported, "user-a", now)[0].remaining_ratio(),
        Some(0.75)
    );
    reported["config"]["currentPeriod"]["type"] = json!("USAGE_PERIOD_TYPE_MONTHLY");
    assert_eq!(
        grok::parse_billing(&reported, "user-a", now)[0].window,
        "monthly"
    );
    assert!(grok::parse_billing(result, "", now).is_empty());
}

#[tokio::test]
async fn grok_initializes_then_reads_billing_without_inference_and_rejects_account_changes() {
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let program = sandbox.path().join("fake-grok");
    fs::write(&program, r#"#!/bin/sh
[ "$1 $2" = 'agent stdio' ] || exit 1
while IFS= read -r request; do
  printf '%s\n' "$request" >> "$GROK_HOME/requests"
  case "$request" in
    *'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"_x.ai/billing"'*) cat "$GROK_HOME/billing-response" ;;
    *) exit 1 ;;
  esac
done
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/grok.json")).unwrap();
    fs::write(
        sandbox.path().join("billing-response"),
        format!("{}\n", fixture["acp_responses"][0]["response"]),
    )
    .unwrap();
    fs::write(
        sandbox.path().join("auth.json"),
        json!({"issuer::client":fixture["identity"]["entry"]}).to_string(),
    )
    .unwrap();
    let context = crate::agents::AgentSession {
        agent_program: Some(program),
        state: sandbox.path().join("state"),
        parent_session_id: None,
        project: "sandbox".into(),
        id: ws_core::ids::Ulid::new().to_string(),
        home: sandbox.path().into(),
        grok_home: Some(sandbox.path().into()),
        codex_home: None,
        claude_config_dir: None,
        ws_home: sandbox.path().join("ws"),
        ws_bin: "/fake/ws".into(),
        spawned: false,
        prompt: None,
        arguments: vec![],
    };
    let prepared =
        crate::agents::prepare("grok", crate::agents::Mode::Headless, &context, None).unwrap();
    let file = prepared
        .files
        .iter()
        .find(|file| file.path.ends_with("quota-binding.json"))
        .unwrap();
    let binding: grok::Binding = serde_json::from_slice(&file.contents).unwrap();
    assert!(!String::from_utf8_lossy(&file.contents).contains("redacted-email"));
    prepared.write_files().unwrap();
    let mut reader = grok::GrokReader::new(binding);
    let readings = reader.read(Arc::new(|| 1_792_186_404_245)).await.unwrap();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].window, "weekly");
    assert_eq!(readings[0].used_ratio, None);
    let requests: Vec<Value> = fs::read_to_string(sandbox.path().join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "initialize");
    assert_eq!(requests[0]["params"]["protocolVersion"], 1);
    assert_eq!(requests[1], fixture["acp_requests"][0]);
    fs::write(
        sandbox.path().join("auth.json"),
        r#"{"issuer::client":{"user_id":"another-account"}}"#,
    )
    .unwrap();
    assert!(reader.read(Arc::new(|| 1_792_186_404_246)).await.is_err());
    assert_eq!(
        fs::read_to_string(sandbox.path().join("requests"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let resumed =
        crate::agents::prepare("grok", crate::agents::Mode::Headless, &context, None).unwrap();
    assert_eq!(
        resumed
            .files
            .iter()
            .find(|f| f.path == file.path)
            .unwrap()
            .contents,
        file.contents
    );
}

#[test]
fn token_estimates_deduplicate_model_records_and_keep_unknown_denominators() {
    use std::fs;
    use ws_core::{
        device::Device,
        ids::Ulid,
        telemetry_log::{Signal, TelemetryLog},
    };
    let sandbox = tempfile::tempdir().unwrap();
    let device = Device::init(sandbox.path(), "test-host").unwrap();
    let log = TelemetryLog::new(sandbox.path(), device.device_id);
    let session = Ulid::new().to_string();
    let other = Ulid::new().to_string();
    for (id, account) in [(&session, "account-a"), (&other, "account-b")] {
        let path = sandbox.path().join("sessions").join(id);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("quota-binding.json"),
            json!({"runner":"claude","account_id":account}).to_string(),
        )
        .unwrap();
    }
    let model = |id: &str, span: &str, at: u64, input: u64, output: u64| {
        json!({
            "resourceSpans":[{"resource":{"attributes":[{"key":sc::SESSION_ID,"value":{"stringValue":id}}]},
            "scopeSpans":[{"spans":[{"name":sc::MODEL_CALL_SPAN,"traceId":id,"spanId":span,
            "startTimeUnixNano":(at - 1).to_string(),"endTimeUnixNano":at.to_string(),"attributes":[
                {"key":sc::GEN_AI_INPUT_TOKENS,"value":{"intValue":input.to_string()}},
                {"key":sc::GEN_AI_OUTPUT_TOKENS,"value":{"intValue":output.to_string()}},
                {"key":sc::GEN_AI_CACHE_READ_INPUT_TOKENS,"value":{"intValue":"999"}},
                {"key":sc::GEN_AI_REQUEST_MODEL,"value":{"stringValue":"model-a"}}
            ]}]}]}]
        })
    };
    let first = model(&session, "call-a", 2_000_000_000, 1000, 100);
    for payload in [
        first.clone(),
        first,
        model(&session, "call-b", 3_000_000_000, 2000, 200),
        model(&other, "call-c", 3_000_000_000, 9000, 900),
        model(&session, "old", 999_000_000, 8000, 800),
        model(&session, "next-window", 5_000_000_000, 7000, 700),
    ] {
        log.append(Signal::Traces, vec![payload], 4000).unwrap();
    }
    let missing = Reading {
        runner: "claude".into(),
        account_id: "account-a".into(),
        bucket: None,
        window: "five_hour".into(),
        duration_secs: Some(4),
        start_ms: Some(1000),
        reset_ms: Some(5000),
        source: Source::Reported,
        source_name: "status-line".into(),
        observed_at_ms: 4000,
        received_at_ms: 4000,
        used_ratio: None,
        tokens: None,
        coverage: None,
    };
    let mut reported = missing.clone();
    reported.used_ratio = Some(0.25);
    let estimates = estimator::estimate_missing(
        sandbox.path(),
        vec![missing.clone(), reported.clone()],
        4000,
    )
    .unwrap();
    // A valid report for this window takes precedence over token estimates.
    assert!(estimates.iter().all(|r| r.source == Source::Reported));
    let estimate = estimator::estimate_missing(sandbox.path(), vec![missing], 4000)
        .unwrap()
        .remove(0);
    assert_eq!(estimate.tokens, Some(3300));
    assert_eq!(estimate.source, Source::Estimated);
    assert_eq!(estimate.remaining_ratio(), None);
    let coverage = estimate.coverage.as_deref().unwrap();
    assert!(coverage.contains("2 model records"));
    assert!(coverage.contains("1000..5000"));
    assert!(coverage.contains("other devices"));
    assert!(coverage.contains("incomplete"));
    let body = estimate.event_body();
    assert_eq!(body[sc::RUNNER_USAGE_TOKENS], 3300);
    assert_eq!(body[sc::RUNNER_USAGE_SOURCE], "estimated");
    assert!(body.get(sc::RUNNER_USAGE_REMAINING_RATIO).is_none());
    assert!(body.get(sc::RUNNER_USAGE_USED_RATIO).is_none());
}

#[tokio::test]
async fn grok_collector_discovers_billing_and_persists_an_estimate_with_coverage() {
    use crate::sessions::{CreateSession, Sessions};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{Arc, Mutex},
    };
    let sandbox = tempfile::tempdir().unwrap();
    let program = sandbox.path().join("grok");
    fs::write(
        &program,
        r#"#!/bin/sh
IFS= read -r request || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
IFS= read -r request || exit 1
printf '%s\n' "$request" > "$GROK_HOME/billing-request"
cat "$GROK_HOME/billing-response"
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/spikes/s3/grok.json")).unwrap();
    fs::write(
        sandbox.path().join("billing-response"),
        format!("{}\n", fixture["acp_responses"][0]["response"]),
    )
    .unwrap();
    fs::write(
        sandbox.path().join("auth.json"),
        r#"{"issuer::client":{"user_id":"account-a","auth_mode":"oidc"}}"#,
    )
    .unwrap();
    let state = sandbox.path().join("state");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    let binding = grok::Binding {
        runner: "grok".into(),
        account_id: Some("account-a".into()),
        identity: Some(grok::Identity {
            user_id: "account-a".into(),
            principal_type: None,
            principal_id: None,
            team_id: None,
        }),
        grok_home: sandbox.path().into(),
        program,
    };
    fs::write(
        state
            .join("sessions")
            .join(session.id.to_string())
            .join("quota-binding.json"),
        serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    let log = ws_core::log::SessionLog::new(&state, session.id, session.device_id);
    let mut changes = sessions.changes.subscribe();
    let store = Arc::new(Mutex::new(sessions));
    let collector = Collector::new(Arc::new(|| 1_792_186_404_245));
    let service = Service::start_with_collector(store.clone(), collector).unwrap();
    wait_for_usage(&log, &mut changes, 1).await;
    store.lock().unwrap().shutdown.send_replace(true);
    service.shutdown().await;
    let entries = log.read().unwrap().entries;
    let event = entries
        .iter()
        .find(|entry| entry.event_type == "runner.usage.updated")
        .unwrap();
    assert_eq!(event.body[sc::RUNNER_USAGE_WINDOW], "weekly");
    assert_eq!(event.body[sc::RUNNER_USAGE_SOURCE], "estimated");
    assert_eq!(
        event.body[sc::RUNNER_USAGE_WINDOW_START],
        1_791_972_561_510_u64
    );
    assert_eq!(event.body[sc::RUNNER_USAGE_TOKENS], 0);
    assert!(
        event.body[sc::RUNNER_USAGE_COVERAGE]
            .as_str()
            .unwrap()
            .contains("incomplete")
    );
    assert!(event.body.get(sc::RUNNER_USAGE_REMAINING_RATIO).is_none());
    let request: Value =
        serde_json::from_slice(&fs::read(sandbox.path().join("billing-request")).unwrap()).unwrap();
    assert_eq!(request["method"], "_x.ai/billing");
}

#[tokio::test]
async fn unavailable_quota_sources_still_persist_token_estimates() {
    use crate::sessions::{CreateSession, Sessions};
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    let sandbox = tempfile::tempdir().unwrap();
    let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    let binding = binding::Binding {
        runner: "codex".into(),
        account_id: Some("launch-account".into()),
        codex_home: sandbox.path().join("absent-home"),
        program: sandbox.path().join("absent-program"),
        resume_id: None,
    };
    fs::write(
        sandbox
            .path()
            .join("sessions")
            .join(session.id.to_string())
            .join("quota-binding.json"),
        serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    let log = ws_core::log::SessionLog::new(sandbox.path(), session.id, session.device_id);
    let mut changes = sessions.changes.subscribe();
    let store = Arc::new(Mutex::new(sessions));
    let service = Service::start_with_collector(
        store.clone(),
        Collector::new(Arc::new(|| 1_792_186_404_245)),
    )
    .unwrap();
    wait_for_usage(&log, &mut changes, 2).await;
    store.lock().unwrap().shutdown.send_replace(true);
    service.shutdown().await;
    let events: Vec<_> = log
        .read()
        .unwrap()
        .entries
        .into_iter()
        .filter(|entry| entry.event_type == "runner.usage.updated")
        .collect();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event.body[sc::RUNNER_USAGE_SOURCE] == "estimated")
    );
    assert!(
        events
            .iter()
            .all(|event| event.body[sc::RUNNER_ACCOUNT_ID] == "launch-account")
    );
    assert!(
        events
            .iter()
            .all(|event| event.body.get(sc::RUNNER_USAGE_REMAINING_RATIO).is_none())
    );
}

#[tokio::test]
async fn grok_rejects_an_account_change_during_the_billing_request() {
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
    let sandbox = tempfile::tempdir().unwrap();
    let program = sandbox.path().join("grok");
    fs::write(&program, r#"#!/bin/sh
IFS= read -r request || exit 1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
IFS= read -r request || exit 1
printf '%s\n' '{"issuer::client":{"user_id":"account-b","auth_mode":"oidc"}}' > "$GROK_HOME/auth.json"
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"config":{"creditUsagePercent":0}}}'
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        sandbox.path().join("auth.json"),
        r#"{"issuer::client":{"user_id":"account-a","auth_mode":"oidc"}}"#,
    )
    .unwrap();
    let mut reader = grok::GrokReader::new(grok::Binding {
        runner: "grok".into(),
        account_id: Some("account-a".into()),
        identity: Some(grok::Identity {
            user_id: "account-a".into(),
            principal_type: None,
            principal_id: None,
            team_id: None,
        }),
        grok_home: sandbox.path().into(),
        program,
    });
    assert!(reader.read(Arc::new(|| 100)).await.is_err());
}

#[test]
fn grok_keeps_unverified_or_ambiguous_authentication_unknown() {
    use crate::agents::{AgentSession, Mode, prepare};
    use std::fs;
    let sandbox = tempfile::tempdir().unwrap();
    for auth in [
        json!({"issuer::client":{"user_id":"a","auth_mode":"unverified"}}),
        json!({"issuer::client":{"user_id":"a"}}),
        json!({"issuer::one":{"user_id":"a","auth_mode":"oidc"},"issuer::two":{"user_id":"b","auth_mode":"oidc"}}),
    ] {
        fs::write(sandbox.path().join("auth.json"), auth.to_string()).unwrap();
        let context = AgentSession {
            agent_program: None,
            state: sandbox.path().join("state"),
            parent_session_id: None,
            project: "sandbox".into(),
            id: ws_core::ids::Ulid::new().to_string(),
            home: sandbox.path().into(),
            grok_home: Some(sandbox.path().into()),
            codex_home: None,
            claude_config_dir: None,
            ws_home: sandbox.path().join("ws"),
            ws_bin: "/fake/ws".into(),
            spawned: false,
            prompt: None,
            arguments: vec![],
        };
        let prepared = prepare("grok", Mode::Headless, &context, None).unwrap();
        let file = prepared
            .files
            .iter()
            .find(|file| file.path.ends_with("quota-binding.json"))
            .unwrap();
        let binding: Value = serde_json::from_slice(&file.contents).unwrap();
        assert!(binding["account_id"].is_null());
    }
}
