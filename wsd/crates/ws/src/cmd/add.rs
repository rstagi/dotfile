use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: Option<String>, agent: String, detach: bool) -> Result<()> {
    let target = match target {
        Some(target) => target,
        None => {
            let output = std::process::Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .output()?;
            if !output.status.success() {
                return Err("add: not inside a workspace (pass <repo/branch>)".into());
            }
            String::from_utf8(output.stdout)?.trim_end().to_owned()
        }
    };
    let body = client::request(
        socket,
        Method::POST,
        "/v1/workspaces/add",
        serde_json::to_vec(&json!({"target": target, "agent": agent}))?,
    )
    .await?;
    let response: serde_json::Value = serde_json::from_slice(&body)?;
    super::new::focus(
        response["window_id"]
            .as_str()
            .ok_or("The workspace response has no window id.")?,
        detach,
    )
}
