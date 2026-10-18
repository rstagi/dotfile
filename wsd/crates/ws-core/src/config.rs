//! Local daemon configuration from the state directory.

use serde::Deserialize;
use std::{fs, io, path::Path};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub limits: SpawnLimits,
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpawnLimits {
    pub max_children: usize,
    pub max_depth: usize,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryConfig {
    /// Zero selects an available port. The daemon publishes its bound endpoint.
    pub port: u16,
    pub retention_days: u64,
}

impl Config {
    /// Reads config.toml. A missing file uses the default values.
    pub fn load(state: &Path) -> io::Result<Self> {
        let path = state.join("config.toml");
        match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Cannot parse {}: {error}", path.display()),
                )
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
}

impl Default for SpawnLimits {
    fn default() -> Self {
        Self {
            max_children: 8,
            max_depth: 3,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            port: 0,
            retention_days: 30,
        }
    }
}
