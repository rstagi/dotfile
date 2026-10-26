//! Read-only quota snapshots from durable session events.

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ws_core::{log::LogEntry, semconv as sc};

use super::{
    Source,
    labels::{self, Homes, Labels},
};
use crate::{
    api::{self, ApiError, Store},
    sessions::Sessions,
};

type WindowKey = (String, String, Option<String>, String);
type BucketKey = (String, String, Option<String>);

/// Account labels can change at a new login. A snapshot reads the login files at most this often.
const LABEL_TTL_MS: u64 = 60_000;

/// Usage readings by window. Appends keep it current, and a rebuild clears it.
#[derive(Debug, Default)]
pub(crate) struct UsageIndex {
    windows: BTreeMap<WindowKey, Vec<LogEntry>>,
    /// The newest observation time of a reported value per bucket. A pruned reading still counts.
    reported: BTreeMap<BucketKey, u64>,
    labels: Option<(u64, Labels)>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Usage {
    pub windows: Vec<Window>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Window {
    pub runner: String,
    pub account_id: Option<String>,
    /// A readable name of the account, for example its email. Absent when the account is not
    /// the current login of the runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
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
    // The history grows with each reading. Load it once, then keep only readings that can win.
    if sessions.usage.is_none() {
        let mut index = UsageIndex::default();
        for entry in sessions.registry.events_of_type("runner.usage.updated")? {
            index.observe(&entry);
        }
        sessions.usage = Some(index);
    }
    let index = sessions.usage.as_mut().unwrap();
    let latest = index.select(now);
    let labels = index.labels(now).clone();
    let windows: Vec<_> = latest
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
                account_label: labels.get(&(runner.clone(), account.clone())).cloned(),
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
    let mut windows = collapse(windows);
    for runner in ["claude", "codex", "grok"] {
        if !windows.iter().any(|window| window.runner == runner) {
            windows.push(Window {
                runner: runner.into(),
                account_id: None,
                account_label: None,
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

/// Removes rows that only repeat what the provider reports.
/// A row without a bucket is a fallback (a failed read or an old rollout). It goes when the
/// provider names a bucket for the same account.
fn collapse(windows: Vec<Window>) -> Vec<Window> {
    let account = |w: &Window| (w.runner.clone(), w.account_id.clone());
    let bucketed: std::collections::HashSet<_> = windows
        .iter()
        .filter(|w| w.bucket.is_some())
        .map(account)
        .collect();
    windows
        .into_iter()
        .filter(|w| w.bucket.is_some() || !bucketed.contains(&account(w)))
        .collect()
}

impl UsageIndex {
    /// The labels of the current logins, read at most once per `LABEL_TTL_MS`.
    fn labels(&mut self, now: u64) -> &Labels {
        if self
            .labels
            .as_ref()
            .is_none_or(|(at, _)| now.saturating_sub(*at) >= LABEL_TTL_MS)
        {
            let labels = Homes::from_env()
                .map(|homes| labels::current(&homes))
                .unwrap_or_default();
            self.labels = Some((now, labels));
        }
        &self.labels.as_ref().unwrap().1
    }

    pub(crate) fn observe(&mut self, entry: &LogEntry) {
        if entry.event_type != "runner.usage.updated" {
            return;
        }
        let body = &entry.body;
        let (Some(runner), Some(account), Some(window)) = (
            body[sc::RUNNER].as_str(),
            body[sc::RUNNER_ACCOUNT_ID].as_str(),
            body[sc::RUNNER_USAGE_WINDOW].as_str(),
        ) else {
            return;
        };
        let bucket = (
            runner.to_owned(),
            account.to_owned(),
            body[sc::RUNNER_USAGE_BUCKET].as_str().map(str::to_owned),
        );
        // Only a valid value counts: a ratio in range, observed before the log wrote it.
        if reports_value(entry) && order(entry).0 <= entry.ts_ms {
            let at = self.reported.entry(bucket.clone()).or_default();
            *at = (*at).max(order(entry).0);
        }
        let key = (bucket.0, bucket.1, bucket.2, window.into());
        self.windows.entry(key).or_default().push(entry.clone());
    }

    /// Selects the reading of each window. Drops readings that can never win again.
    /// A window goes when a newer reported value of its bucket arrives without it: the provider
    /// no longer lists that window. A live reported value stays, because the collector does not
    /// repeat an unchanged reading.
    fn select(&mut self, now: u64) -> BTreeMap<WindowKey, LogEntry> {
        let mut latest = BTreeMap::new();
        for (key, entries) in &mut self.windows {
            let mut best = 0;
            for (index, entry) in entries.iter().enumerate().skip(1) {
                if supersedes(entry, &entries[best], now) {
                    best = index;
                }
            }
            let newest = (0..entries.len())
                .reduce(|kept, index| {
                    if order(&entries[index]) > order(&entries[kept]) {
                        index
                    } else {
                        kept
                    }
                })
                .unwrap();
            let (runner, account, bucket, _) = key;
            let omitted = self
                .reported
                .get(&(runner.clone(), account.clone(), bucket.clone()))
                .is_some_and(|at| *at > order(&entries[newest]).0);
            if !omitted || (live_report(&entries[best], now) && reports_value(&entries[best])) {
                latest.insert(key.clone(), entries[best].clone());
            }
            // A report stops being live at its reset and never becomes live again.
            let mut index = 0;
            entries.retain(|entry| {
                let keep = index == best || index == newest || live_report(entry, now);
                index += 1;
                keep
            });
        }
        latest
    }
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

fn reports_value(entry: &LogEntry) -> bool {
    entry.body[sc::RUNNER_USAGE_SOURCE] == "reported"
        && entry.body[sc::RUNNER_USAGE_REMAINING_RATIO]
            .as_f64()
            .is_some_and(|ratio| (0.0..=1.0).contains(&ratio))
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
            ts_ms: NOW,
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
    fn index_keeps_each_reading_that_can_still_win() {
        let window = |mut entry: LogEntry| {
            entry.body[sc::RUNNER] = "claude".into();
            entry.body[sc::RUNNER_ACCOUNT_ID] = "account-a".into();
            entry.body[sc::RUNNER_USAGE_WINDOW] = "five_hour".into();
            entry
        };
        let report = window(reported(NOW + 60_000));
        let estimate = window(estimated());
        let mut index = UsageIndex::default();
        index.observe(&report);
        index.observe(&estimate);
        let selected = |index: &mut UsageIndex, now| {
            index
                .select(now)
                .into_values()
                .map(|entry| entry.body[sc::RUNNER_USAGE_SOURCE].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(selected(&mut index, NOW), ["reported"]);
        // After the reset, the newer estimate must still be available.
        assert_eq!(selected(&mut index, NOW + 60_000), ["estimated"]);
    }

    fn window(
        bucket: Option<&str>,
        name: &str,
        remaining: Option<f64>,
        source: Source,
        at: u64,
    ) -> Window {
        Window {
            runner: "codex".into(),
            account_id: Some("account-a".into()),
            account_label: None,
            bucket: bucket.map(str::to_owned),
            window: name.into(),
            remaining_ratio: remaining,
            source: Some(source),
            observed_at_ms: Some(at),
            age_secs: Some(1),
            status: if remaining.is_some() {
                "available"
            } else {
                "unknown"
            }
            .into(),
            reset_ms: None,
            tokens: None,
            coverage: None,
        }
    }

    fn keys(windows: &[Window]) -> Vec<(Option<&str>, &str)> {
        windows
            .iter()
            .map(|w| (w.bucket.as_deref(), w.window.as_str()))
            .collect()
    }

    #[test]
    fn collapse_drops_bucketless_fallbacks_once_the_provider_names_a_bucket() {
        let windows = vec![
            window(None, "five_hour", None, Source::Estimated, 10),
            window(None, "weekly", None, Source::Estimated, 10),
            window(Some("codex"), "weekly", Some(0.92), Source::Reported, 20),
        ];
        assert_eq!(keys(&collapse(windows)), [(Some("codex"), "weekly")]);
    }

    fn codex(name: &str, source: &str, at: u64, remaining: Option<f64>) -> LogEntry {
        let mut entry = entry(json!({
            (sc::RUNNER): "codex",
            (sc::RUNNER_ACCOUNT_ID): "account-a",
            (sc::RUNNER_USAGE_BUCKET): "codex",
            (sc::RUNNER_USAGE_WINDOW): name,
            (sc::RUNNER_USAGE_SOURCE): source,
            (sc::RUNNER_USAGE_OBSERVED_AT): at,
            (sc::RUNNER_USAGE_WINDOW_RESET): NOW + 60_000,
        }));
        if let Some(remaining) = remaining {
            entry.body[sc::RUNNER_USAGE_REMAINING_RATIO] = remaining.into();
        }
        entry
    }

    fn selected(index: &mut UsageIndex) -> Vec<String> {
        index.select(NOW).into_keys().map(|key| key.3).collect()
    }

    #[test]
    fn select_drops_a_window_that_a_newer_report_of_the_bucket_omits() {
        let mut index = UsageIndex::default();
        // One collection lists both windows. The five-hour value is an estimate.
        index.observe(&codex("weekly", "reported", 10, Some(0.9)));
        index.observe(&codex("five_hour", "estimated", 11, None));
        assert_eq!(selected(&mut index), ["five_hour", "weekly"]);
        // The next collection lists only the weekly window.
        index.observe(&codex("weekly", "reported", 20, Some(0.8)));
        assert_eq!(selected(&mut index), ["weekly"]);
    }

    #[test]
    fn select_keeps_an_unchanged_live_report_that_the_collector_did_not_repeat() {
        let mut index = UsageIndex::default();
        index.observe(&codex("five_hour", "reported", 10, Some(0.5)));
        index.observe(&codex("weekly", "reported", 20, Some(0.8)));
        assert_eq!(selected(&mut index), ["five_hour", "weekly"]);
    }

    #[test]
    fn expired_report_loses_to_newer_estimate() {
        let report = reported(NOW - 500);
        let estimate = estimated();
        assert!(supersedes(&estimate, &report, NOW));
        assert!(!supersedes(&report, &estimate, NOW));
    }
}
