use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: &str, detach: bool) -> Result<()> {
    let target = if target == "me" {
        super::whoami::current(socket).await?.id.to_string()
    } else {
        target.to_owned()
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
