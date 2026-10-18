//! Token estimates use completed canonical model spans and frozen account bindings.

use super::{Reading, Source};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
};
use ws_core::{
    device::Device,
    semconv as sc,
    telemetry_log::{Signal, TelemetryLog},
};

type Accounts = HashMap<String, (String, String)>;

struct ModelRecord {
    session: String,
    at_ms: u64,
    tokens: u64,
}

/// Estimates missing values. Token counts alone cannot establish a subscription denominator.
pub fn estimate_missing(
    state: &Path,
    mut readings: Vec<Reading>,
    now: u64,
) -> io::Result<Vec<Reading>> {
    let reported: HashSet<_> = readings
        .iter()
        .filter(|reading| usable(reading, now))
        .map(window_key)
        .collect();
    if readings
        .iter()
        .all(|reading| reported.contains(&window_key(reading)))
    {
        return Ok(readings);
    }
    let accounts = accounts(state)?;
    let records = model_records(state)?;
    for reading in &mut readings {
        if reported.contains(&window_key(reading)) {
            continue;
        }
        let start = reading.start_ms.or_else(|| {
            reading
                .reset_ms
                .zip(reading.duration_secs)
                .and_then(|(reset, secs)| reset.checked_sub(secs.checked_mul(1000)?))
        });
        let end = reading.reset_ms;
        // A rolling interval measures observed tokens without inventing a provider reset.
        let lower = start.or_else(|| {
            reading
                .duration_secs
                .map(|secs| now.saturating_sub(secs.saturating_mul(1000)))
        });
        let upper = end.unwrap_or_else(|| now.saturating_add(1));
        let matches: Vec<_> = records
            .iter()
            .filter(|record| {
                lower.is_some_and(|lower| record.at_ms >= lower)
                    && record.at_ms < upper
                    && record.at_ms <= now
                    && accounts
                        .get(&record.session)
                        .is_some_and(|(runner, account)| {
                            runner == &reading.runner && account == &reading.account_id
                        })
            })
            .collect();
        reading.source = Source::Estimated;
        reading.source_name = "token-telemetry".into();
        reading.observed_at_ms = now;
        reading.received_at_ms = now;
        reading.used_ratio = None;
        reading.tokens = lower.map(|_| {
            matches
                .iter()
                .fold(0_u64, |total, record| total.saturating_add(record.tokens))
        });
        let interval = match (lower, end) {
            (Some(lower), Some(end)) => format!("{lower}..{end}"),
            (Some(lower), None) => {
                format!("{lower}..{upper} rolling interval, provider boundaries unknown")
            }
            _ => "unknown window boundaries".into(),
        };
        let observed = matches
            .iter()
            .map(|record| record.at_ms)
            .min()
            .zip(matches.iter().map(|record| record.at_ms).max())
            .map(|(first, last)| format!("{first}..{last}"))
            .unwrap_or_else(|| "none".into());
        reading.coverage = Some(format!(
            "Local canonical telemetry, incomplete: {} model records, window {interval}, observed {observed}. Excludes other devices, sessions outside ws and missing telemetry. No calibrated denominator.",
            matches.len()
        ));
    }
    Ok(readings)
}

/// Retains the launch identity when a reporting source is unavailable.
pub(crate) fn missing_for_session(
    state: &Path,
    id: ws_core::ids::SessionId,
    now: u64,
) -> Vec<Reading> {
    let binding = fs::read(
        state
            .join("sessions")
            .join(id.to_string())
            .join("quota-binding.json"),
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let Some(binding) = binding else {
        return Vec::new();
    };
    let Some(account) = binding["account_id"].as_str().filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    match binding["runner"].as_str() {
        Some("codex") => {
            super::codex::parse_app_server(&serde_json::json!({"accountId":account}), account, now)
                .unwrap_or_default()
        }
        Some("claude") => super::claude::parse_status_line(&Value::Null, account, now),
        Some("grok") => super::grok::parse_billing(&Value::Null, account, now),
        _ => Vec::new(),
    }
}

fn usable(reading: &Reading, now: u64) -> bool {
    reading.source == Source::Reported
        && reading.used_ratio.is_some()
        && reading.reset_ms.is_none_or(|reset| reset > now)
        && reading.start_ms.is_none_or(|start| start <= now)
}

fn window_key(reading: &Reading) -> (String, String, Option<String>, String, Option<u64>) {
    (
        reading.runner.clone(),
        reading.account_id.clone(),
        reading.bucket.clone(),
        reading.window.clone(),
        reading.reset_ms,
    )
}

fn accounts(state: &Path) -> io::Result<Accounts> {
    let entries = match fs::read_dir(state.join("sessions")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };
    let mut accounts = HashMap::new();
    for entry in entries {
        let entry = entry?;
        let binding = fs::read(entry.path().join("quota-binding.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        if let Some(binding) = binding
            && let (Some(runner), Some(account)) =
                (binding["runner"].as_str(), binding["account_id"].as_str())
        {
            accounts.insert(
                entry.file_name().to_string_lossy().into_owned(),
                (runner.into(), account.into()),
            );
        }
    }
    Ok(accounts)
}

fn model_records(state: &Path) -> io::Result<Vec<ModelRecord>> {
    let device = Device::load(state)?;
    let mut records = HashMap::new();
    for record in TelemetryLog::new(state, device.device_id).read()? {
        if record.signal != Signal::Traces {
            continue;
        }
        for resource in record.payload["resourceSpans"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for scope in resource["scopeSpans"].as_array().into_iter().flatten() {
                for span in scope["spans"].as_array().into_iter().flatten() {
                    // Native logs and native spans can describe the same call. Only the
                    // Phase 43 canonical span contributes to this account total.
                    if span["name"] != sc::MODEL_CALL_SPAN {
                        continue;
                    }
                    let Some(session) = attribute(&resource["resource"], sc::SESSION_ID)
                        .or_else(|| attribute(span, sc::SESSION_ID))
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    let (Some(trace), Some(id), Some(end), Some(input), Some(output)) = (
                        span["traceId"].as_str().filter(|id| !id.is_empty()),
                        span["spanId"].as_str().filter(|id| !id.is_empty()),
                        number(&span["endTimeUnixNano"]).filter(|time| *time > 0),
                        attribute(span, sc::GEN_AI_INPUT_TOKENS).and_then(number),
                        attribute(span, sc::GEN_AI_OUTPUT_TOKENS).and_then(number),
                    ) else {
                        continue;
                    };
                    // Canonical input already includes cache tokens. Output includes reasoning.
                    records.insert(
                        (trace.to_owned(), id.to_owned()),
                        ModelRecord {
                            session: session.into(),
                            at_ms: end / 1_000_000,
                            tokens: input.saturating_add(output),
                        },
                    );
                }
            }
        }
    }
    Ok(records.into_values().collect())
}

fn attribute<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let value = &value["attributes"]
        .as_array()?
        .iter()
        .find(|attribute| attribute["key"] == key)?["value"];
    value.get("stringValue").or_else(|| value.get("intValue"))
}

fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}
