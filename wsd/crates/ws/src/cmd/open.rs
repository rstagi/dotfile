use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: &str, detach: bool) -> Result<()> {
    let target = if target == "me" {
        super::whoami::current(socket).await?.id.to_string()
    } else {
        normalize_target(target)?
    };
    let body = client::request(
        socket,
        Method::POST,
        "/v1/workspaces/open",
        serde_json::to_vec(&json!({
            "target": target, "parent_session_id": super::caller(socket).await?
        }))?,
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

pub(super) fn normalize_target(target: &str) -> Result<String> {
    let path = super::new::expand_home(target)?;
    if path.is_dir() {
        return path
            .canonicalize()?
            .into_os_string()
            .into_string()
            .map_err(|_| "The directory path is not UTF-8.".into());
    }
    Ok(target.to_owned())
}
