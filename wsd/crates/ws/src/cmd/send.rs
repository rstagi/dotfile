use crate::{Result, client, cmd};
use hyper::Method;
use serde_json::json;
use std::{io::Read, path::Path, process::ExitCode};

pub async fn run(socket: &Path, target: &str, message: &str, json: bool) -> Result<ExitCode> {
    if json {
        let body = message_body(message)?;
        super::mcp::print(socket, "session_send", json!({"target":target,"body":body})).await?;
        return Ok(ExitCode::from(75));
    }
    let recipient = client::session(socket, target).await?;
    let sender = cmd::caller(socket).await?;
    let body = message_body(message)?;
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

fn message_body(message: &str) -> Result<String> {
    let mut body = message.to_owned();
    if message == "-" {
        body.clear();
        std::io::stdin().read_to_string(&mut body)?;
    }
    if body.is_empty() {
        return Err("send: empty message".into());
    }
    Ok(body)
}
