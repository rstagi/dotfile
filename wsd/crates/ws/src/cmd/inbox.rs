use crate::{Result, client, cmd};
use hyper::Method;
use std::path::Path;
use ws_daemon::bus::Message;

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let session = cmd::caller(socket).await?;
    let path = session
        .map(|id| format!("/v1/inbox?session_id={id}"))
        .unwrap_or_else(|| "/v1/inbox".into());
    let body = client::request(socket, Method::GET, &path, Vec::new()).await?;
    let messages: Vec<Message> = serde_json::from_slice(&body)?;
    if json {
        println!("{}", serde_json::to_string(&messages)?);
    } else {
        for message in messages {
            let sender = message
                .from_alias
                .map(|alias| format!("#{alias}"))
                .unwrap_or_else(|| message.content.from_session.to_string());
            println!("[ws {sender}]");
            for line in message.content.body.split('\n') {
                if line.starts_with("[ws ") {
                    print!(" ");
                }
                println!("{line}");
            }
        }
    }
    Ok(())
}
