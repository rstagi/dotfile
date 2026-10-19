//! Grok billing periods retain missing percentages as unknown values.

use super::{Clock, ReadFuture, Reader, Reading, Source};
use crate::agents::{AgentSession, GeneratedFile};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub principal_type: Option<String>,
    pub principal_id: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Binding {
    pub runner: String,
    pub account_id: Option<String>,
    pub identity: Option<Identity>,
    pub grok_home: PathBuf,
    pub program: PathBuf,
}

pub struct GrokReader {
    binding: Binding,
}

struct BillingProcess {
    child: Child,
    pgid: u32,
    armed: bool,
}

impl Reader for GrokReader {
    fn read(&mut self, clock: Clock) -> ReadFuture<'_> {
        Box::pin(async move {
            let Some(account) = self.binding.account_id.as_deref() else {
                return Ok(Vec::new());
            };
            if self.binding.identity.is_none()
                || identity(&self.binding.grok_home) != self.binding.identity
            {
                return Err(io::Error::other(
                    "The Grok account differs from the launch account.",
                ));
            }
            let result = read_billing(&self.binding).await?;
            if identity(&self.binding.grok_home) != self.binding.identity {
                return Err(io::Error::other(
                    "The Grok account changed during the billing request.",
                ));
            }
            Ok(parse_billing(&result, account, clock()))
        })
    }
}

impl GrokReader {
    pub fn new(binding: Binding) -> Self {
        Self { binding }
    }
}

/// Parses the billing result. No missing field receives a default percentage.
pub fn parse_billing(result: &Value, account: &str, received: u64) -> Vec<Reading> {
    if account.is_empty() {
        return Vec::new();
    }
    let config = &result["config"];
    let period = &config["currentPeriod"];
    let window = match period["type"].as_str() {
        Some("USAGE_PERIOD_TYPE_WEEKLY") => "weekly",
        Some("USAGE_PERIOD_TYPE_MONTHLY") => "monthly",
        Some(other) => other,
        None => "unknown",
    };
    let start_ms = timestamp(&period["start"]);
    let reset_ms = timestamp(&period["end"]);
    let duration_secs = start_ms
        .zip(reset_ms)
        .and_then(|(start, end)| end.checked_sub(start))
        .filter(|duration| *duration > 0)
        .map(|duration| duration / 1000);
    let used_ratio = config["creditUsagePercent"]
        .as_f64()
        .filter(|used| used.is_finite() && (0.0..=100.0).contains(used))
        .map(|used| used / 100.0)
        .filter(|_| reset_ms.is_none_or(|reset| reset > received))
        .filter(|_| start_ms.is_none_or(|start| start <= received));
    vec![Reading {
        runner: "grok".into(),
        account_id: account.into(),
        bucket: None,
        window: window.into(),
        duration_secs,
        start_ms,
        reset_ms,
        source: Source::Reported,
        source_name: "acp-billing".into(),
        observed_at_ms: received,
        received_at_ms: received,
        used_ratio,
        tokens: None,
        coverage: None,
    }]
}

/// Keeps the original account and principal context when a session resumes.
pub(crate) fn prepare_file(session: &AgentSession) -> io::Result<GeneratedFile> {
    let path = session
        .state
        .join("sessions")
        .join(&session.id)
        .join("quota-binding.json");
    let contents = match fs::read(&path) {
        Ok(contents) => {
            let binding: Binding = serde_json::from_slice(&contents)?;
            if binding.runner != "grok" {
                return Err(io::Error::other(
                    "The launch quota binding belongs to another runner.",
                ));
            }
            contents
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let grok_home = session
                .grok_home
                .clone()
                .unwrap_or_else(|| session.home.join(".grok"));
            let identity = identity(&grok_home);
            let account_id = identity.as_ref().map(|identity| {
                // The principal separates team allowances that share one login.
                serde_json::to_string(identity).expect("Identity contains only strings.")
            });
            serde_json::to_vec(&Binding {
                runner: "grok".into(),
                account_id,
                identity,
                grok_home,
                program: session
                    .agent_program
                    .clone()
                    .unwrap_or_else(|| "grok".into()),
            })?
        }
        Err(error) => return Err(error),
    };
    Ok(GeneratedFile { path, contents })
}

fn timestamp(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
}

// Ambiguous authentication entries cannot establish the quota owner.
fn identity(home: &Path) -> Option<Identity> {
    let auth: Value = serde_json::from_slice(&fs::read(home.join("auth.json")).ok()?).ok()?;
    let mut entries = auth.as_object()?.values().filter_map(|entry| {
        if entry["auth_mode"] != "oidc" {
            return None;
        }
        let user_id = entry["user_id"].as_str().filter(|id| !id.is_empty())?;
        Some(Identity {
            user_id: user_id.into(),
            principal_type: entry["principal_type"].as_str().map(str::to_owned),
            principal_id: entry["principal_id"].as_str().map(str::to_owned),
            team_id: entry["team_id"].as_str().map(str::to_owned),
        })
    });
    let first = entries.next()?;
    if entries.any(|entry| entry != first) {
        return None;
    }
    Some(first)
}

async fn read_billing(binding: &Binding) -> io::Result<Value> {
    let child = Command::new(&binding.program)
        .args(["agent", "stdio"])
        .env("GROK_HOME", &binding.grok_home)
        .current_dir(&binding.grok_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let mut process = BillingProcess {
        pgid: child.id().unwrap(),
        child,
        armed: true,
    };
    let result = tokio::time::timeout(Duration::from_secs(30), exchange(&mut process.child))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "The Grok billing request exceeded its deadline.",
            )
        })
        .and_then(|result| result);
    process.kill_group();
    let _ = process.child.wait().await;
    process.armed = false;
    result
}

async fn exchange(child: &mut Child) -> io::Result<Value> {
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap().take(4 * 1024 * 1024));
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}});
    input
        .write_all(format!("{initialize}\n").as_bytes())
        .await?;
    let initialized = response(&mut output, 1).await?;
    if initialized["protocolVersion"] != 1 {
        return Err(io::Error::other(
            "The Grok billing protocol version is unsupported.",
        ));
    }
    let request = json!({"jsonrpc":"2.0","id":2,"method":"_x.ai/billing","params":{}});
    input.write_all(format!("{request}\n").as_bytes()).await?;
    response(&mut output, 2).await
}

async fn response(output: &mut (impl AsyncBufReadExt + Unpin), id: u64) -> io::Result<Value> {
    loop {
        let mut line = String::new();
        if output.read_line(&mut line).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "The Grok billing process closed its output.",
            ));
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|_| io::Error::other("The Grok billing response is invalid."))?;
        if value["id"] != id {
            continue;
        }
        return value
            .get("result")
            .filter(|result| result.is_object())
            .cloned()
            .ok_or_else(|| io::Error::other("The Grok billing process rejected the request."));
    }
}

impl BillingProcess {
    fn kill_group(&self) {
        // SAFETY: This child owns the process group and has not exited through wait.
        unsafe {
            libc::kill(-(self.pgid as i32), libc::SIGKILL);
        }
    }
}

impl Drop for BillingProcess {
    fn drop(&mut self) {
        if self.armed {
            self.kill_group();
        }
    }
}
