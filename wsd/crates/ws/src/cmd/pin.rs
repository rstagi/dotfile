use crate::{Result, client};
use hyper::Method;
use serde_json::json;
use std::path::Path;

pub async fn run(socket: &Path, target: &str) -> Result<()> {
    let target = super::open::normalize_target(target)?;
    client::request(
        socket,
        Method::POST,
        "/v1/workspaces/pin",
        serde_json::to_vec(&json!({
            "target": target, "parent_session_id": super::caller(socket).await?
        }))?,
    )
    .await?;
    Ok(())
}
