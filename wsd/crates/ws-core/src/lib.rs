//! Shared types for the WS daemon and CLI.

use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize, Serialize)]
pub struct PingResponse {
    pub version: String,
    pub uptime_secs: u64,
}
