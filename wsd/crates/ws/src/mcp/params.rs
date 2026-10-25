use std::{collections::BTreeMap, path::PathBuf};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// Session ULID, alias, name, or me.
    pub target: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    /// New unique name. Start with a letter. Use letters, digits, dots, underscores or hyphens.
    pub name: String,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    Codex,
    Grok,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub name: Option<String>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub command: Vec<String>,
    /// Start an interactive session in this directory, or set the headless process directory.
    pub cwd: Option<PathBuf>,
    /// Start an interactive session in a new scratch directory.
    #[serde(default)]
    pub scratch: bool,
    /// Start an interactive session for this repository. With branch, create a git worktree and a tmux workspace.
    pub repo: Option<PathBuf>,
    /// Branch for the new worktree. Use it only with repo.
    pub branch: Option<String>,
    /// Agent for an interactive session. The default is claude.
    pub agent: Option<Agent>,
    /// First message to the agent of an interactive session.
    pub prompt: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub stdin: Option<PathBuf>,
    pub stdout: Option<PathBuf>,
    pub stderr: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Send {
    pub target: String,
    pub body: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// Session that needs the dependency.
    pub target: String,
    /// Session that must satisfy the dependency.
    pub dependency: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wait {
    /// Use target for one session, or targets for several sessions.
    pub target: Option<String>,
    pub targets: Option<Vec<WaitTarget>>,
    /// Use any or all for targets. The default is all.
    pub mode: Option<WaitMode>,
    /// Match only events with a sequence number above this value. Use this field with target.
    pub after_seq: Option<u64>,
    /// Use done, exit, state=<state>, or event=<type>.
    pub condition: String,
    /// Maximum wait in seconds. Omit this field for an indefinite wait.
    pub timeout: Option<u32>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitTarget {
    pub target: String,
    /// Match only events with a sequence number above this value. The default is zero.
    pub after_seq: Option<u64>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WaitMode {
    Any,
    All,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tail {
    pub target: String,
    pub stream: Option<Stream>,
    /// Maximum output bytes. The default is 8192. The maximum is 65536.
    #[schemars(range(min = 1, max = 65536))]
    pub max_bytes: Option<u32>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub command: Vec<String>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub phase: Phase,
    pub repo: PathBuf,
    pub base: String,
    #[serde(default)]
    pub caps_required: Vec<String>,
    /// Maximum process runtime in seconds.
    pub max_runtime: u64,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Loop {
    /// Directory that contains the Loop state.json file.
    pub loop_dir: PathBuf,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Session ULID, alias, name, or me.
    pub target: String,
    /// Event type. It must start with loop.
    #[serde(rename = "type")]
    pub event_type: String,
    /// JSON object with the event data.
    pub body: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Log {
    /// Session ULID, alias, name, or me.
    pub target: String,
    /// Keep entries whose type starts with this prefix.
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    /// Keep entries with a sequence number above this value.
    pub since: Option<u64>,
}
