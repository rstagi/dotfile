//! Local daemon configuration from the state directory.

use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub agent: AgentConfig,
    pub limits: SpawnLimits,
    pub naming: NamingConfig,
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub priority: AgentPriority,
}

/// The macOS QoS clamp for agent processes. Child processes inherit the clamp.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentPriority {
    /// Uses all cores when they are free. Interactive processes get the CPU first.
    #[default]
    Utility,
    /// Uses only the efficiency cores and throttles disk I/O.
    Background,
    /// Applies no clamp.
    Normal,
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
    /// OTLP/HTTP base URLs. Each sink receives the full retained window.
    pub sinks: Vec<String>,
    pub native_tracing: bool,
    pub content: bool,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NamingConfig {
    /// The model that improves the first automatic name. `off` keeps the slug of the prompt.
    pub model: String,
    /// The Claude Code executable. A bare name resolves through PATH.
    pub program: String,
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

impl AgentPriority {
    /// Returns the argv prefix that starts a program with this clamp. Normal priority and
    /// systems without taskpolicy return an empty prefix.
    pub fn prefix(self) -> Vec<String> {
        const TASKPOLICY: &str = "/usr/sbin/taskpolicy";
        let clamp = match self {
            Self::Utility => "utility",
            Self::Background => "background",
            Self::Normal => return Vec::new(),
        };
        if !cfg!(target_os = "macos") || !Path::new(TASKPOLICY).exists() {
            return Vec::new();
        }
        [TASKPOLICY, "-c", clamp].map(String::from).to_vec()
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
            sinks: Vec::new(),
            native_tracing: false,
            content: false,
        }
    }
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            model: "haiku".into(),
            program: "claude".into(),
        }
    }
}
