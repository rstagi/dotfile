//! Read-only quota snapshots from durable session events.

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ws_core::{log::LogEntry, semconv as sc};

use super::Source;
use crate::{
    api::{self, ApiError, Store},
    sessions::Sessions,
};

type WindowKey = (String, String, Option<String>, String);

#[derive(Debug, Deserialize, Serialize)]
pub struct Usage {
    pub windows: Vec<Window>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Window {
    pub runner: String,
    pub account_id: Option<String>,
    pub bucket: Option<String>,
    pub window: String,
    pub remaining_ratio: Option<f64>,
    pub source: Option<Source>,
    pub observed_at_ms: Option<u64>,
    pub age_secs: Option<u64>,
    pub status: String,
    pub reset_ms: Option<u64>,
    pub tokens: Option<u64>,
    pub coverage: Option<String>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/usage", get(usage))
}

async fn usage(State(store): State<Store>) -> Result<Json<Usage>, ApiError> {
    Ok(Json(
        api::call(store, |sessions| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_millis() as u64;
            snapshot(sessions, now)
        })
        .await?,
    ))
}

fn snapshot(sessions: &mut Sessions, now: u64) -> crate::sessions::Result<Usage> {
    let mut latest: BTreeMap<WindowKey, LogEntry> = BTreeMap::new();
    for entry in sessions.registry.events_of_type("runner.usage.updated")? {
        let body = &entry.body;
        let (Some(runner), Some(account), Some(window)) = (
            body[sc::RUNNER].as_str(),
            body[sc::RUNNER_ACCOUNT_ID].as_str(),
            body[sc::RUNNER_USAGE_WINDOW].as_str(),
        ) else {
            continue;
        };
        let key = (
            runner.into(),
            account.into(),
            body[sc::RUNNER_USAGE_BUCKET].as_str().map(str::to_owned),
            window.into(),
        );
        if latest
            .get(&key)
            .is_none_or(|previous| supersedes(&entry, previous, now))
        {
            latest.insert(key, entry);
        }
    }
    let mut windows: Vec<_> = latest
        .into_iter()
        .map(|((runner, account, bucket, window), entry)| {
            let body = entry.body;
            let observed_at_ms = body[sc::RUNNER_USAGE_OBSERVED_AT].as_u64();
            let age_secs = observed_at_ms
                .and_then(|at| now.checked_sub(at))
                .map(|age| age / 1000);
            let reset_ms = body[sc::RUNNER_USAGE_WINDOW_RESET].as_u64();
            let unavailable = reset_ms.is_some_and(|reset| reset <= now)
                || body[sc::RUNNER_USAGE_WINDOW_START]
                    .as_u64()
                    .is_some_and(|start| start > now);
            let source =
                serde_json::from_value::<Source>(body[sc::RUNNER_USAGE_SOURCE].clone()).ok();
            let remaining_ratio = body[sc::RUNNER_USAGE_REMAINING_RATIO]
                .as_f64()
                .filter(|ratio| (0.0..=1.0).contains(ratio))
                .filter(|_| !unavailable && age_secs.is_some() && source.is_some());
            Window {
                runner,
                account_id: Some(account),
                bucket,
                window,
                remaining_ratio,
                source,
                observed_at_ms,
                age_secs,
                status: if unavailable {
                    "unavailable"
                } else if remaining_ratio.is_some() {
                    "available"
                } else {
                    "unknown"
                }
                .into(),
                reset_ms,
                tokens: body[sc::RUNNER_USAGE_TOKENS].as_u64(),
                coverage: body[sc::RUNNER_USAGE_COVERAGE].as_str().map(str::to_owned),
            }
        })
        .collect();
    for runner in ["claude", "codex", "grok"] {
        if !windows.iter().any(|window| window.runner == runner) {
            windows.push(Window {
                runner: runner.into(),
                account_id: None,
                bucket: None,
                window: "unknown".into(),
                remaining_ratio: None,
                source: None,
                observed_at_ms: None,
                age_secs: None,
                status: "unavailable".into(),
                reset_ms: None,
                tokens: None,
                coverage: None,
            });
        }
    }
    windows.sort_by(|a, b| {
        (&a.runner, &a.account_id, &a.bucket, &a.window).cmp(&(
            &b.runner,
            &b.account_id,
            &b.bucket,
            &b.window,
        ))
    });
    Ok(Usage { windows })
}

/// Returns true when `entry` replaces `previous` as the selected reading of one window.
/// A report whose window has not reset wins over any estimate (semconv: an estimate
/// must not silently replace a reported reading). Otherwise the newest reading wins.
fn supersedes(entry: &LogEntry, previous: &LogEntry, now: u64) -> bool {
    match (live_report(entry, now), live_report(previous, now)) {
        (true, false) => true,
        (false, true) => false,
        _ => order(entry) > order(previous),
    }
}

fn live_report(entry: &LogEntry, now: u64) -> bool {
    entry.body[sc::RUNNER_USAGE_SOURCE] == "reported"
        && entry.body[sc::RUNNER_USAGE_WINDOW_RESET]
            .as_u64()
            .is_none_or(|reset| reset > now)
}

fn order(entry: &LogEntry) -> (u64, bool, u64, u64) {
    let body: &Value = &entry.body;
    (
        body[sc::RUNNER_USAGE_OBSERVED_AT].as_u64().unwrap_or(0),
        body[sc::RUNNER_USAGE_SOURCE] == "reported",
        body["received_at_ms"].as_u64().unwrap_or(0),
        entry.ts_ms,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use ws_core::ids::Ulid;

    use super::*;

    const NOW: u64 = 1_000_000;

    fn entry(body: Value) -> LogEntry {
        LogEntry {
            event_id: Ulid::nil(),
            session_id: Ulid::nil(),
            epoch: 0,
            seq: 0,
            prev_hash: String::new(),
            ts_ms: 0,
            event_type: "runner.usage.updated".into(),
            body,
            device_id: Ulid::nil(),
            sig: String::new(),
        }
    }

    fn reported(reset: u64) -> LogEntry {
        entry(json!({
            (sc::RUNNER_USAGE_SOURCE): "reported",
            (sc::RUNNER_USAGE_OBSERVED_AT): NOW - 2_000,
            (sc::RUNNER_USAGE_REMAINING_RATIO): 0.75,
            (sc::RUNNER_USAGE_WINDOW_RESET): reset,
        }))
    }

    fn estimated() -> LogEntry {
        entry(json!({
            (sc::RUNNER_USAGE_SOURCE): "estimated",
            (sc::RUNNER_USAGE_OBSERVED_AT): NOW - 1_000,
        }))
    }

    #[test]
    fn unexpired_report_wins_over_newer_estimate() {
        let report = reported(NOW + 60_000);
        let estimate = estimated();
        assert!(
            !supersedes(&estimate, &report, NOW),
            "A newer estimate must not replace an unexpired reported window."
        );
        assert!(
            supersedes(&report, &estimate, NOW),
            "An unexpired report must replace an estimate."
        );
    }

    #[test]
    fn expired_report_loses_to_newer_estimate() {
        let report = reported(NOW - 500);
        let estimate = estimated();
        assert!(supersedes(&estimate, &report, NOW));
        assert!(!supersedes(&report, &estimate, NOW));
    }
}
