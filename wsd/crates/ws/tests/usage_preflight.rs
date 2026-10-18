mod common;

use std::fs;

use serde_json::{Value, json};
use ws_core::{device::Device, log::SessionLog};
use ws_daemon::sessions::{CreateSession, Sessions};

const MODELS: &str = r#"
CHAIN_TASK=("codex:@sol")
CHAIN_REVIEW_MAX_GPT=("codex:@astra")
CHAIN_REVIEW_MAX_CLAUDE=("claude:fable")
CHAIN_REVIEW_MAX_GROK=("grok:default")
CHAIN_REVIEW_MAX_FIX=("claude:opus")
CHAIN_REVIEW_MAX_FINAL=("claude:opus")
LOOP_REVIEW_TIER_DEFAULT=max
LOOP_REVIEW_ROUNDS_DEFAULT=3
LOOP_REVIEW_INNER_DEFAULT=2
LOOP_TIMEOUT_TASK=1800
LOOP_TIMEOUT_REVIEW=1800
LOOP_TIMEOUT_REMEDIATE=1800
"#;

const PLAN: &str = "\
## Phases
### Phase 1 — Completed work `[status: done]`
- **Estimate:** 100–200 min
### Phase 2 — Work `[status: todo]`
- **Repository:** `acme/api`
- **Estimate:** 20–40 min
### Phase 3 — Review `[status: todo]` `[kind: pr-review]` `[review: max]` `[rounds: 3]`
";

#[test]
fn preflight_warns_for_review_demand_and_suggests_a_runner_with_more_quota() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    seed(
        home.path(),
        &[
            ("codex", "five_hour", Some(0.01)),
            ("claude", "five_hour", Some(0.9)),
            ("grok", "five_hour", Some(0.5)),
        ],
    );
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    assert!(common::get(&socket, "/v1/usage").starts_with("HTTP/1.1 200"));
    fs::write(home.path().join("plan.md"), PLAN).unwrap();
    fs::write(home.path().join("loop-models.conf"), MODELS).unwrap();
    let mut command = common::ws(home.path(), &socket);
    let output = command
        .current_dir(home.path())
        .args(["usage", "--preflight", "--plan", "plan.md", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let codex = report["demands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["runner"] == "codex")
        .unwrap();
    assert_eq!(codex["phase"], "3");
    assert_eq!(codex["before_minutes"], 40.0);
    assert_eq!(codex["review_minutes"], 180.0);
    assert_eq!(codex["remaining_minutes"], 3.0);
    assert_eq!(codex["warning"], true);
    assert_eq!(codex["alternative"], "claude");
    let output = common::ws(home.path(), &socket)
        .current_dir(home.path())
        .args(["usage", "--preflight", "--plan", "plan.md"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Warning: codex"), "{text}");
    assert!(text.contains("review phase 3"), "{text}");
    assert!(text.contains("Consider claude"), "{text}");
}

#[test]
fn preflight_uses_a_weekly_only_runner_and_excludes_unknown_alternatives() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    seed(
        home.path(),
        &[
            ("codex", "weekly", Some(0.001)),
            ("claude", "five_hour", Some(0.99)),
            ("claude", "weekly", None),
            ("grok", "five_hour", Some(0.5)),
        ],
    );
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    fs::write(home.path().join("plan.md"), PLAN).unwrap();
    fs::write(home.path().join("loop-models.conf"), MODELS).unwrap();
    let output = common::ws(home.path(), &socket)
        .current_dir(home.path())
        .args(["usage", "--preflight", "--plan", "plan.md", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let codex = report["demands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["runner"] == "codex")
        .unwrap();
    assert_eq!(codex["remaining_minutes"], 10.08);
    assert_eq!(codex["warning"], true);
    assert_eq!(codex["alternative"], "grok");
}

#[test]
fn preflight_reads_changed_chains_and_scopes_demand_to_the_selected_milestone() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    seed(
        home.path(),
        &[
            ("codex", "weekly", Some(0.8)),
            ("claude", "five_hour", Some(0.01)),
        ],
    );
    let _daemon = common::Daemon::start(common::ws(home.path(), &socket), &socket);
    let plan = format!(
        "## Milestone 1 — Shipped\n{}\n## Milestone 2 — Next\n{}",
        PLAN.replace("## Phases\n", ""),
        PLAN.replace("## Phases\n", "").replace("20–40", "10–20")
    );
    fs::write(home.path().join("plan.md"), plan).unwrap();
    fs::write(
        home.path().join("models.conf"),
        MODELS.replace(
            "CHAIN_TASK=(\"codex:@sol\")",
            "CHAIN_TASK=(\"claude:sonnet\")",
        ),
    )
    .unwrap();
    let output = common::ws(home.path(), &socket)
        .current_dir(home.path())
        .args([
            "usage",
            "--preflight",
            "--plan",
            "plan.md",
            "--models-conf",
            "models.conf",
            "--milestone",
            "2",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let demands = report["demands"].as_array().unwrap();
    assert_eq!(demands.len(), 3);
    let claude = demands.iter().find(|d| d["runner"] == "claude").unwrap();
    assert_eq!(claude["before_minutes"], 20.0);
    assert_eq!(claude["review_minutes"], 270.0);
    assert_eq!(claude["alternative"], "codex");
    let grok = demands.iter().find(|d| d["runner"] == "grok").unwrap();
    assert!(grok["remaining_minutes"].is_null());
}

#[test]
fn preflight_reports_no_quota_data_without_blocking_on_transport_or_plan_errors() {
    let home = common::SandboxRoot::new();
    let socket = home.path().join("wsd.sock");
    fs::write(home.path().join("plan.md"), PLAN).unwrap();
    fs::write(home.path().join("loop-models.conf"), MODELS).unwrap();
    common::create_stale_socket(&socket);
    for plan in ["plan.md", "missing.md"] {
        let output = common::ws(home.path(), &socket)
            .current_dir(home.path())
            .args(["usage", "--preflight", "--plan", plan])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("no quota data"));
    }
}

fn seed(home: &std::path::Path, windows: &[(&str, &str, Option<f64>)]) {
    let state = home.join("state");
    let mut sessions = Sessions::open(&state, 1).unwrap();
    let session = sessions.create(CreateSession::default()).unwrap();
    drop(sessions);
    let device = Device::load(&state).unwrap();
    let log = SessionLog::new(&state, session.id, device.device_id);
    for (runner, window, remaining) in windows {
        log.append(
            1,
            "runner.usage.updated",
            json!({
                "ws.runner": runner, "ws.runner.account.id": "test-account",
                "ws.runner.usage.window": window,
                "ws.runner.usage.remaining_ratio": remaining,
                "ws.runner.usage.source": "reported",
                "ws.runner.usage.observed_at": 1000,
            }),
        )
        .unwrap();
    }
    // Keep any accidental dispatcher call inside the sandbox.
    fs::create_dir_all(home.join("fake-agents")).unwrap();
    let fake = home.join("fake-agents/ws");
    fs::write(&fake, "#!/bin/sh\nexit 99\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(fake, fs::Permissions::from_mode(0o755)).unwrap();
}
