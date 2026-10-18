//! Account-bound runner quota collection.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, io, pin::Pin, sync::Arc};
use ws_core::semconv as sc;

pub mod binding;
pub mod codex;
mod collector;
pub(crate) use collector::Service;
pub use collector::{Collector, Requests, Trigger};

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = io::Result<Vec<Reading>>> + Send + 'a>>;

/// A runner source returns account-bound snapshots with explicit provenance.
pub trait Reader: Send {
    fn read(&mut self, clock: Clock) -> ReadFuture<'_>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Reported,
    Estimated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Reading {
    pub runner: String,
    pub account_id: String,
    pub bucket: Option<String>,
    pub window: String,
    pub duration_secs: Option<u64>,
    pub start_ms: Option<u64>,
    pub reset_ms: Option<u64>,
    pub source: Source,
    pub source_name: String,
    pub observed_at_ms: u64,
    pub received_at_ms: u64,
    pub used_ratio: Option<f64>,
    pub tokens: Option<u64>,
    pub coverage: Option<String>,
}

impl Reading {
    /// Returns the normalized body of a durable runner.usage.updated event.
    pub fn event_body(&self) -> Value {
        let mut body = json!({
            sc::RUNNER: self.runner, sc::RUNNER_ACCOUNT_ID: self.account_id,
            sc::RUNNER_USAGE_WINDOW: self.window, sc::RUNNER_USAGE_SOURCE: self.source,
            sc::RUNNER_USAGE_SOURCE_NAME: self.source_name,
            sc::RUNNER_USAGE_OBSERVED_AT: self.observed_at_ms,
            "received_at_ms": self.received_at_ms,
        });
        for (key, value) in [
            (sc::RUNNER_USAGE_BUCKET, json!(self.bucket)),
            (sc::RUNNER_USAGE_WINDOW_DURATION, json!(self.duration_secs)),
            (sc::RUNNER_USAGE_WINDOW_START, json!(self.start_ms)),
            (sc::RUNNER_USAGE_WINDOW_RESET, json!(self.reset_ms)),
            (sc::RUNNER_USAGE_USED_RATIO, json!(self.used_ratio)),
            (
                sc::RUNNER_USAGE_REMAINING_RATIO,
                json!(self.remaining_ratio()),
            ),
            (sc::RUNNER_USAGE_TOKENS, json!(self.tokens)),
            (sc::RUNNER_USAGE_COVERAGE, json!(self.coverage)),
        ] {
            if !value.is_null() {
                body[key] = value;
            }
        }
        body
    }
    pub fn remaining_ratio(&self) -> Option<f64> {
        self.used_ratio.map(|used| 1.0 - used)
    }
}

#[cfg(test)]
mod tests;
