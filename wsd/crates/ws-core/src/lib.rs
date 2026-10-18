//! Shared types and state storage for the WS daemon and CLI.

pub mod config;
pub mod device;
pub mod ids;
pub mod lockfile;
pub mod log;
pub mod registry;
pub mod semconv;
pub mod state;

use std::{fs, io, os::unix::fs::DirBuilderExt, path::Path};

use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize, Serialize)]
pub struct PingResponse {
    pub version: String,
    pub uptime_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    Stop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HookRecord {
    pub event: HookEvent,
    pub window_id: String,
}

/// Creates private state directories and persists their parent entries.
pub(crate) fn create_state_directory(path: &Path) -> io::Result<()> {
    let missing: Vec<_> = path
        .ancestors()
        .take_while(|directory| !directory.is_dir())
        .filter(|directory| !directory.as_os_str().is_empty())
        .collect();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    for directory in missing {
        let parent = directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
