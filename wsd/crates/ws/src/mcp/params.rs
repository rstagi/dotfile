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
pub struct Create {
    pub name: Option<String>,
    pub workspace_id: Option<String>,
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
    pub target: String,
    /// Use done, exit, state=<state>, or event=<type>.
    pub condition: String,
    /// Maximum wait in seconds. Omit this field for an indefinite wait.
    pub timeout: Option<u32>,
}
