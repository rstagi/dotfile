use crate::{Result, client, cmd};
use hyper::Method;
use serde_json::json;
use std::path::Path;
use ws_core::ids::EventId;

pub async fn run(socket: &Path, event_ids: Vec<EventId>) -> Result<()> {
    let session = cmd::caller(socket).await?;
    client::request(
        socket,
        Method::POST,
        "/v1/ack",
        serde_json::to_vec(&json!({"session_id":session, "event_ids":event_ids}))?,
    )
    .await?;
    Ok(())
}
