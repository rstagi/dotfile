use crate::{Result, client, cmd};
use hyper::Method;
use serde_json::json;
use std::{io::Read, path::Path, process::ExitCode};

pub async fn run(socket: &Path, target: &str, message: &str) -> Result<ExitCode> {
    let recipient = client::session(socket, target).await?;
    let sender = cmd::caller(socket).await?;
    let mut body = message.to_owned();
    if message == "-" {
        body.clear();
        std::io::stdin().read_to_string(&mut body)?;
    }
    if body.is_empty() {
        return Err("send: empty message".into());
    }
    client::request(
        socket,
        Method::POST,
        "/v1/messages",
        serde_json::to_vec(
            &json!({"target":recipient.id, "body":body, "parent_session_id":sender}),
        )?,
    )
    .await?;
    eprintln!(
        "ws: send: The message is queued for session #{}.",
        recipient.alias
    );
    Ok(ExitCode::from(75))
}
