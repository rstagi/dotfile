//! Codex app-server and passive rollout quota sources.

use super::{Clock, ReadFuture, Reader, Reading, Source, binding::Binding};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{self, BufRead},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};
use ws_core::ids::SessionId;

pub struct CodexReader {
    binding: Binding,
    state: PathBuf,
    session_id: SessionId,
}

struct AppServer {
    child: Child,
    pgid: u32,
    armed: bool,
}

impl Reader for CodexReader {
    fn read(&mut self, clock: Clock) -> ReadFuture<'_> {
        Box::pin(async move {
            let Some(account) = self.binding.account_id.as_deref() else {
                return Ok(Vec::new());
            };
            let state = self.state.clone();
            let session_id = self.session_id;
            let binding = self.binding.clone();
            let receipt = clock();
            let mut readings = tokio::task::spawn_blocking(move || {
                read_rollouts(&binding, &state, session_id, receipt)
            })
            .await
            .map_err(io::Error::other)
            .and_then(|result| result)
            .unwrap_or_default();
            match read_limits(&self.binding)
                .await
                .and_then(|result| parse_app_server(&result, account, clock()))
            {
                Ok(reported) => readings.extend(reported),
                Err(error) if readings.is_empty() => return Err(error),
                Err(_) => {}
            }
            Ok(readings)
        })
    }
}

impl CodexReader {
    pub fn new(binding: Binding, state: PathBuf, session_id: SessionId) -> Self {
        Self {
            binding,
            state,
            session_id,
        }
    }
}

/// Parses one RPC result. The launch account cannot change with the current credentials.
pub fn parse_app_server(
    result: &Value,
    account: &str,
    received_at_ms: u64,
) -> io::Result<Vec<Reading>> {
    if account.is_empty() || result["accountId"].as_str() != Some(account) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The quota account differs from the launch account.",
        ));
    }
    let mut readings = Vec::new();
    if let Some(buckets) = result["rateLimitsByLimitId"]
        .as_object()
        .filter(|map| !map.is_empty())
    {
        for (id, bucket) in buckets {
            readings.extend(parse_bucket(
                bucket,
                Some(id),
                account,
                "app-server",
                received_at_ms,
                received_at_ms,
                true,
            ));
        }
    } else {
        readings.extend(parse_bucket(
            &result["rateLimits"],
            None,
            account,
            "app-server",
            received_at_ms,
            received_at_ms,
            true,
        ));
    }
    Ok(readings)
}

/// Uses the rollout event time for reading age and the collector time for receipt.
pub fn parse_rollout(
    event: &Value,
    account: &str,
    received_at_ms: u64,
) -> io::Result<Vec<Reading>> {
    if event["type"] != "event_msg"
        || event["payload"]["type"] != "token_count"
        || !event["payload"]["rate_limits"].is_object()
    {
        return Ok(Vec::new());
    }
    let observed = event["timestamp"]
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The rollout quota timestamp is missing or invalid.",
            )
        })?;
    Ok(parse_bucket(
        &event["payload"]["rate_limits"],
        None,
        account,
        "rollout",
        observed,
        received_at_ms,
        false,
    ))
}

fn read_rollouts(
    binding: &Binding,
    state: &Path,
    session_id: SessionId,
    receipt: u64,
) -> io::Result<Vec<Reading>> {
    let entries = ws_core::log::SessionLog::new(state, session_id, ws_core::ids::Ulid::nil())
        .read()?
        .entries;
    let conversation = entries
        .iter()
        .rev()
        .filter(|entry| entry.event_type == "hook.received")
        .filter(|entry| entry.body["payload"]["agent_id"].is_null())
        .filter(|entry| {
            !entry.body["payload"]["hook_event_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("Subagent"))
        })
        .find_map(|entry| entry.body["payload"]["session_id"].as_str())
        .or(binding.resume_id.as_deref());
    let Some(conversation) = conversation
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    rollout_paths(
        &binding.codex_home.join("sessions"),
        &format!("-{conversation}.jsonl"),
        &mut paths,
    )?;
    let mut latest = std::collections::BTreeMap::new();
    for path in paths {
        for line in std::io::BufReader::new(File::open(path)?).lines() {
            let line = line?;
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let Ok(readings) = parse_rollout(
                &event,
                binding.account_id.as_deref().unwrap_or_default(),
                receipt,
            ) else {
                continue;
            };
            for reading in readings {
                let key = (reading.bucket.clone(), reading.window.clone());
                let old: Option<&Reading> = latest.get(&key);
                if old.is_none_or(|old| old.observed_at_ms <= reading.observed_at_ms) {
                    latest.insert(key, reading);
                }
            }
        }
    }
    Ok(latest.into_values().collect())
}

fn rollout_paths(directory: &Path, suffix: &str, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            rollout_paths(&entry.path(), suffix, paths)?;
        } else if kind.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(suffix))
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

async fn read_limits(binding: &Binding) -> io::Result<Value> {
    let child = Command::new(&binding.program)
        .arg("app-server")
        .env("CODEX_HOME", &binding.codex_home)
        .current_dir(&binding.codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let mut server = AppServer {
        pgid: child.id().unwrap(),
        child,
        armed: true,
    };
    let result = tokio::time::timeout(Duration::from_secs(30), exchange(&mut server.child))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "The quota app-server did not respond before the deadline.",
            )
        })
        .and_then(|result| result);
    server.kill_group();
    let _ = server.child.wait().await;
    server.armed = false;
    result
}

async fn exchange(child: &mut Child) -> io::Result<Value> {
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap().take(4 * 1024 * 1024));
    let init = serde_json::json!({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "wsd", "version": ws_core::VERSION}}});
    input.write_all(format!("{init}\n").as_bytes()).await?;
    response(&mut output, 1).await?;
    input.write_all(b"{\"method\":\"initialized\",\"params\":{}}\n{\"id\":2,\"method\":\"account/rateLimits/read\"}\n").await?;
    response(&mut output, 2).await
}

async fn response(output: &mut (impl AsyncBufReadExt + Unpin), id: u64) -> io::Result<Value> {
    loop {
        let mut line = String::new();
        if output.read_line(&mut line).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "The quota app-server closed its output.",
            ));
        }
        let value: Value = serde_json::from_str(&line).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The quota app-server response is invalid.",
            )
        })?;
        if value["id"] != id {
            continue;
        }
        return value
            .get("result")
            .cloned()
            .filter(Value::is_object)
            .ok_or_else(|| io::Error::other("The quota app-server rejected the request."));
    }
}

impl AppServer {
    fn kill_group(&self) {
        // SAFETY: This child created the process group and has not been reaped.
        unsafe {
            libc::kill(-(self.pgid as i32), libc::SIGKILL);
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        if self.armed {
            self.kill_group();
        }
    }
}

fn parse_bucket(
    bucket: &Value,
    fallback_id: Option<&String>,
    account: &str,
    source: &str,
    observed: u64,
    received: u64,
    rpc: bool,
) -> Vec<Reading> {
    let field = |camel, snake| if rpc { camel } else { snake };
    let bucket_id = bucket[field("limitId", "limit_id")]
        .as_str()
        .map(str::to_owned)
        .or_else(|| fallback_id.cloned());
    let mut readings = Vec::new();
    for position in ["primary", "secondary"] {
        let value = &bucket[position];
        let Some(duration_secs) = value[field("windowDurationMins", "window_minutes")]
            .as_u64()
            .and_then(|mins| mins.checked_mul(60))
            .filter(|secs| *secs > 0)
        else {
            continue;
        };
        let window = match duration_secs {
            18000 => "five_hour".into(),
            604800 => "weekly".into(),
            other => format!("duration:{other}"),
        };
        let reset_ms = value[field("resetsAt", "resets_at")]
            .as_u64()
            .and_then(|seconds| seconds.checked_mul(1000));
        let used_ratio = value[field("usedPercent", "used_percent")]
            .as_f64()
            .filter(|used| used.is_finite() && (0.0..=100.0).contains(used))
            .map(|used| used / 100.0)
            .filter(|_| reset_ms.is_none_or(|reset| reset > received));
        readings.push(Reading {
            runner: "codex".into(),
            account_id: account.into(),
            bucket: bucket_id.clone(),
            window,
            duration_secs: Some(duration_secs),
            start_ms: None,
            reset_ms,
            source: Source::Reported,
            source_name: source.into(),
            observed_at_ms: observed,
            received_at_ms: received,
            used_ratio,
            tokens: None,
            coverage: None,
        });
    }
    // A bucket without any window keeps both standard windows as unknown values. A bucket that
    // reports a window lists all of its windows: a plan with a weekly limit only has no five-hour
    // window, and an invented one would show as unknown next to the real limit.
    if !readings.is_empty() {
        return readings;
    }
    [("five_hour", 18000), ("weekly", 604800)]
        .into_iter()
        .map(|(window, seconds)| Reading {
            runner: "codex".into(),
            account_id: account.into(),
            bucket: bucket_id.clone(),
            window: window.into(),
            duration_secs: Some(seconds),
            start_ms: None,
            reset_ms: None,
            source: Source::Reported,
            source_name: source.into(),
            observed_at_ms: observed,
            received_at_ms: received,
            used_ratio: None,
            tokens: None,
            coverage: None,
        })
        .collect()
}
