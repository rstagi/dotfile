use std::path::Path;

use serde_json::json;
use ws_daemon::wait::Condition;

use crate::{Result, client};

pub async fn run(socket: &Path, target: &str, condition: &str, timeout: Option<u32>) -> Result<()> {
    Condition::parse(condition)?;
    let session = if target == "me" {
        super::whoami::current(socket).await?
    } else {
        client::session(socket, target).await?
    };
    let caller = super::caller(socket).await?;
    client::request(
        socket,
        hyper::Method::POST,
        &format!("/v1/sessions/{}/wait", session.id),
        serde_json::to_vec(
            &json!({"for":condition, "timeout":timeout, "parent_session_id":caller}),
        )?,
    )
    .await?;
    Ok(())
}
