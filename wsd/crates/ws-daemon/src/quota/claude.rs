//! Claude status-line percentages and stream fractions normalize to used ratios (0..=1).

use super::{Clock, ReadFuture, Reader, Reading, Source};
use crate::agents::{AgentSession, GeneratedFile, Prepared};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
};
use ws_core::{
    ids::SessionId,
    log::{LogEntry, SessionLog},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Binding {
    pub runner: String,
    pub account_id: Option<String>,
}

pub struct ClaudeReader {
    binding: Binding,
    state: PathBuf,
    session_id: SessionId,
    stream_event: Option<Value>,
    stream_readings: Vec<Reading>,
    first_receipt: Option<u64>,
    facts: LogFacts,
}

/// Log facts that each read extends with the new log tail only.
#[derive(Default)]
struct LogFacts {
    offset: u64,
    callbacks: Vec<LogEntry>,
    stdout: Option<String>,
    previous: Vec<Value>,
}

struct Inputs {
    status: Vec<Reading>,
    stream: Option<Value>,
    previous: Vec<Value>,
}

impl Reader for ClaudeReader {
    fn read(&mut self, clock: Clock) -> ReadFuture<'_> {
        Box::pin(async move {
            let Some(account) = self.binding.account_id.clone().filter(|id| !id.is_empty()) else {
                return Ok(Vec::new());
            };
            let state = self.state.clone();
            let id = self.session_id;
            let receipt = clock();
            let first = *self.first_receipt.get_or_insert(receipt);
            let bound_account = account.clone();
            let facts = std::mem::take(&mut self.facts);
            let (inputs, facts) =
                tokio::task::spawn_blocking(move || read_inputs(&bound_account, &state, id, facts))
                    .await
                    .map_err(io::Error::other)??;
            self.facts = facts;
            if inputs.stream != self.stream_event {
                self.stream_readings = inputs
                    .stream
                    .as_ref()
                    .map(|event| parse_stream_event(event, &account, receipt))
                    .unwrap_or_default();
                if self.stream_event.is_none() {
                    for reading in &mut self.stream_readings {
                        if let Some(old) = inputs.previous.iter().rev().find(|body| {
                            body[ws_core::semconv::RUNNER_USAGE_WINDOW] == reading.window
                                && body[ws_core::semconv::RUNNER_USAGE_USED_RATIO]
                                    == serde_json::json!(reading.used_ratio)
                                && body[ws_core::semconv::RUNNER_USAGE_WINDOW_RESET]
                                    == serde_json::json!(reading.reset_ms)
                        }) {
                            reading.observed_at_ms = old
                                [ws_core::semconv::RUNNER_USAGE_OBSERVED_AT]
                                .as_u64()
                                .unwrap_or(receipt);
                        }
                    }
                }
                self.stream_event = inputs.stream;
            }
            let mut readings = parse_windows(&Value::Null, &account, first, false);
            for reading in &mut readings {
                if let Some(latest) = inputs
                    .status
                    .iter()
                    .chain(&self.stream_readings)
                    .filter(|candidate| candidate.window == reading.window)
                    .max_by_key(|candidate| candidate.observed_at_ms)
                {
                    *reading = latest.clone();
                }
                reading.received_at_ms = receipt;
                if reading.reset_ms.is_some_and(|reset| reset <= receipt) {
                    reading.used_ratio = None;
                }
            }
            Ok(readings)
        })
    }
}

impl ClaudeReader {
    pub fn new(binding: Binding, state: PathBuf, session_id: SessionId) -> Self {
        Self {
            binding,
            state,
            session_id,
            stream_event: None,
            stream_readings: Vec::new(),
            first_receipt: None,
            facts: LogFacts::default(),
        }
    }
}

/// Extends the shared launch contract without changing the user's settings.
pub(crate) fn prepare(prepared: &mut Prepared, session: &AgentSession) -> io::Result<()> {
    let settings = prepared
        .files
        .iter_mut()
        .find(|file| file.path.ends_with("claude-settings.json"))
        .ok_or_else(|| io::Error::other("The Claude launch settings are missing."))?;
    let mut value: Value = serde_json::from_slice(&settings.contents)?;
    value["statusLine"] = serde_json::json!({
        "type": "command",
        "command": format!("{} hook quota", shell_words::quote(&session.ws_bin.to_string_lossy()))
    });
    settings.contents = serde_json::to_vec(&value)?;
    let path = session
        .state
        .join("sessions")
        .join(&session.id)
        .join("quota-binding.json");
    let contents = match fs::read(&path) {
        Ok(contents) => {
            let binding: Binding = serde_json::from_slice(&contents)?;
            if binding.runner != "claude" {
                return Err(io::Error::other(
                    "The launch quota binding belongs to another runner.",
                ));
            }
            contents
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let account_id = fs::read(
                session
                    .claude_config_dir
                    .as_ref()
                    .unwrap_or(&session.home)
                    .join(".claude.json"),
            )
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|config| {
                config["oauthAccount"]["accountUuid"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            });
            serde_json::to_vec(&Binding {
                runner: "claude".into(),
                account_id,
            })?
        }
        Err(error) => return Err(error),
    };
    prepared.files.push(GeneratedFile { path, contents });
    Ok(())
}

/// Parses a status-line callback. Receipt time is not a provider measurement timestamp.
pub fn parse_status_line(input: &Value, account: &str, received_at_ms: u64) -> Vec<Reading> {
    parse_windows(&input["rate_limits"], account, received_at_ms, true)
}

/// Ignores other stream records. A rate-limit event can omit either window.
pub fn parse_stream_event(event: &Value, account: &str, received_at_ms: u64) -> Vec<Reading> {
    if event["type"] != "rate_limit_event" {
        return Vec::new();
    }
    parse_windows(
        &event["rate_limit_info"]["unifiedWindows"],
        account,
        received_at_ms,
        false,
    )
}

fn parse_windows(windows: &Value, account: &str, received: u64, percentage: bool) -> Vec<Reading> {
    if account.is_empty() {
        return Vec::new();
    }
    [
        ("five_hour", "five_hour", 18000),
        ("seven_day", "weekly", 604800),
    ]
    .into_iter()
    .map(|(field, window, duration)| {
        let value = &windows[field];
        let reset_ms = value[if percentage { "resets_at" } else { "resetsAt" }]
            .as_u64()
            .and_then(|seconds| seconds.checked_mul(1000));
        let maximum = if percentage { 100.0 } else { 1.0 };
        let used_ratio = value[if percentage {
            "used_percentage"
        } else {
            "utilization"
        }]
        .as_f64()
        .filter(|used| used.is_finite() && (0.0..=maximum).contains(used))
        .map(|used| used / maximum)
        .filter(|_| reset_ms.is_none_or(|reset| reset > received));
        Reading {
            runner: "claude".into(),
            account_id: account.into(),
            bucket: None,
            window: window.into(),
            duration_secs: Some(duration),
            start_ms: None,
            reset_ms,
            source: Source::Reported,
            source_name: if percentage {
                "status-line"
            } else {
                "stream-json"
            }
            .into(),
            observed_at_ms: received,
            received_at_ms: received,
            used_ratio,
            tokens: None,
            coverage: None,
        }
    })
    .collect()
}

fn read_inputs(
    account: &str,
    state: &Path,
    id: SessionId,
    mut facts: LogFacts,
) -> io::Result<(Inputs, LogFacts)> {
    let tail = SessionLog::new(state, id, ws_core::ids::Ulid::nil()).read_after(facts.offset)?;
    if tail.restarted {
        facts = LogFacts::default();
    }
    facts.offset = tail.offset;
    for entry in tail.entries {
        match entry.event_type.as_str() {
            "hook.received" if entry.body["state"] == "quota" => facts.callbacks.push(entry),
            "session.created" => {
                facts.stdout = entry.body["stdout"].as_str().map(str::to_owned);
            }
            "runner.usage.updated"
                if entry.body[ws_core::semconv::RUNNER_USAGE_SOURCE_NAME] == "stream-json"
                    && entry.body[ws_core::semconv::RUNNER_ACCOUNT_ID] == account =>
            {
                facts.previous.push(entry.body);
            }
            _ => {}
        }
    }
    let mut status: Vec<Reading> = Vec::new();
    let mut callbacks: Vec<_> = facts.callbacks.iter().collect();
    callbacks.sort_by_key(|entry| {
        entry.body["observed_at_us"]
            .as_u64()
            .map(|time| time / 1000)
            .unwrap_or(entry.ts_ms)
    });
    for entry in callbacks {
        let observed = entry.body["observed_at_us"]
            .as_u64()
            .map(|time| time / 1000)
            .unwrap_or(entry.ts_ms);
        let mut readings = parse_status_line(&entry.body["payload"], account, observed);
        for reading in &mut readings {
            if let Some(previous) = status.iter().find(|old| {
                old.window == reading.window
                    && old.used_ratio == reading.used_ratio
                    && old.reset_ms == reading.reset_ms
            }) {
                reading.observed_at_ms = previous.observed_at_ms;
            }
        }
        status = readings;
    }
    let stream = facts
        .stdout
        .as_deref()
        .and_then(|path| read_stream(Path::new(path)).ok().flatten());
    let inputs = Inputs {
        status,
        stream,
        previous: facts.previous.clone(),
    };
    Ok((inputs, facts))
}

fn read_stream(path: &Path) -> io::Result<Option<Value>> {
    let mut latest = None;
    for line in io::BufReader::new(fs::File::open(path)?).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event["type"] == "rate_limit_event" {
            latest = Some(event);
        }
    }
    Ok(latest)
}
