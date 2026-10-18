//! Telemetry settings verified by the S2 agent samples.

use std::{
    fs,
    io::{self, Read, Seek},
    net::SocketAddr,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use ws_core::{config::Config, device::Device, ids::Ulid, log::SessionLog, semconv};

use super::{AgentSession, Prepared, TelemetryMetadata};

const CLAUDE_SETTINGS_VERSION: &str = "2.1.302";
const CODEX_SETTINGS_VERSION: &str = "0.159.2";

pub(super) fn extend(
    prepared: &mut Prepared,
    agent: &str,
    session: &AgentSession,
) -> io::Result<()> {
    if !matches!(agent, "claude" | "codex") {
        return Ok(());
    }
    let endpoint = match fs::read_to_string(session.state.join("otlp.endpoint")) {
        Ok(endpoint) => endpoint.trim().trim_end_matches('/').to_owned(),
        // Offline preparation remains available before the local receiver starts.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let address = endpoint
        .strip_prefix("http://")
        .and_then(|s| s.parse::<SocketAddr>().ok());
    if !address.is_some_and(|address| address.ip().is_loopback() && address.port() != 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The telemetry endpoint must use HTTP on localhost.",
        ));
    }
    prepared.unset_env.extend(
        [
            "TRACEPARENT",
            "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA",
            "ENABLE_BETA_TRACING_DETAILED",
            "BETA_TRACING_ENDPOINT",
            "OTEL_LOG_USER_PROMPTS",
            "OTEL_LOG_TOOL_DETAILS",
            "OTEL_LOG_TOOL_CONTENT",
        ]
        .map(str::to_owned),
    );
    if agent == "claude" {
        for signal in ["LOGS", "METRICS", "TRACES"] {
            for setting in ["ENDPOINT", "PROTOCOL", "HEADERS"] {
                prepared
                    .unset_env
                    .push(format!("OTEL_EXPORTER_OTLP_{signal}_{setting}"));
            }
        }
        prepared.unset_env.push("OTEL_EXPORTER_OTLP_HEADERS".into());
    }
    let settings_version = if agent == "claude" {
        CLAUDE_SETTINGS_VERSION
    } else {
        CODEX_SETTINGS_VERSION
    };
    let agent_version = agent_version(agent, session);
    prepared.telemetry = Some(TelemetryMetadata {
        version_covered: agent_version.as_ref().is_some_and(|version| {
            version
                .split_whitespace()
                .any(|part| part == settings_version)
        }),
        agent_version,
        settings_version: settings_version.into(),
    });
    let config = Config::load(&session.state)?.telemetry;
    let device = Device::load(&session.state)?;
    let id: Ulid = session
        .id
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let entries = SessionLog::new(&session.state, id, device.device_id)
        .read()?
        .entries;
    let created = entries
        .iter()
        .find(|entry| entry.event_type == "session.created");
    let parent = session
        .parent_session_id
        .map(|id| id.to_string())
        .or_else(|| {
            created.and_then(|entry| entry.body["parent_session_id"].as_str().map(str::to_owned))
        })
        .unwrap_or_default();
    let project = created
        .and_then(|entry| entry.body["workspace_id"].as_str())
        .unwrap_or(&session.project);
    let resources = [
        (semconv::SESSION_ID, session.id.as_str()),
        (semconv::PARENT_SESSION_ID, parent.as_str()),
        (semconv::DEVICE_ID, &device.device_id.to_string()),
        (semconv::PROJECT, project),
    ]
    .map(|(key, value)| format!("{key}={}", resource_value(value)))
    .join(",");
    prepared
        .env
        .insert("OTEL_RESOURCE_ATTRIBUTES".into(), resources);
    if agent == "claude" {
        for (key, value) in [
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("OTEL_METRICS_EXPORTER", "otlp"),
            ("OTEL_LOGS_EXPORTER", "otlp"),
            ("OTEL_TRACES_EXPORTER", "otlp"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str()),
        ] {
            prepared.env.insert(key.into(), value.into());
        }
    } else {
        for setting in [
            "analytics.enabled=true".into(),
            "otel.log_user_prompt=false".into(),
            exporter("exporter", &endpoint, "logs"),
            exporter("metrics_exporter", &endpoint, "metrics"),
            if config.native_tracing {
                exporter("trace_exporter", &endpoint, "traces")
            } else {
                "otel.trace_exporter=\"none\"".into()
            },
        ] {
            prepared.argv.extend(["-c".into(), setting]);
        }
    }
    if config.native_tracing {
        let trace = u128::from(id);
        let span = (u128::from(Ulid::new()) as u64).max(1);
        prepared.env.insert(
            "TRACEPARENT".into(),
            format!("00-{trace:032x}-{span:016x}-01"),
        );
        if agent == "claude" {
            for (key, value) in [
                ("CLAUDE_CODE_ENHANCED_TELEMETRY_BETA", "1"),
                ("ENABLE_BETA_TRACING_DETAILED", "1"),
                ("BETA_TRACING_ENDPOINT", endpoint.as_str()),
            ] {
                prepared.env.insert(key.into(), value.into());
            }
        }
    }
    Ok(())
}

fn agent_version(agent: &str, session: &AgentSession) -> Option<String> {
    let mut output = tempfile::tempfile().ok()?;
    let mut child = Command::new(
        session
            .agent_program
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new(agent)),
    )
    .arg("--version")
    .env("HOME", &session.home)
    .env("CLAUDE_CONFIG_DIR", session.home.join(".claude"))
    .env("CODEX_HOME", session.home.join(".codex"))
    .env("XDG_CONFIG_HOME", session.home.join(".config"))
    .stdin(Stdio::null())
    .stdout(output.try_clone().ok()?)
    .stderr(Stdio::null())
    .spawn()
    .ok()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    output.rewind().ok()?;
    let mut version = String::new();
    output.take(4096).read_to_string(&mut version).ok()?;
    version
        .lines()
        .next()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
}

fn resource_value(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace(',', "%2C")
        .replace('=', "%3D")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn exporter(setting: &str, endpoint: &str, signal: &str) -> String {
    format!(
        "otel.{setting}={{otlp-http={{endpoint=\"{endpoint}/v1/{signal}\",protocol=\"json\"}}}}"
    )
}
