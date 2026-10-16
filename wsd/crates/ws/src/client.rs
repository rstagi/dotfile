use std::{path::Path, time::Duration};

use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use tokio::{net::UnixStream, time::timeout};
use ws_core::{HookEvent, HookRecord, PingResponse};

use crate::Result;

pub async fn ping(socket: &Path) -> Result<PingResponse> {
    let body = request(socket, Method::GET, "/v1/ping", Vec::new()).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub async fn hook_stop(socket: &Path, window_id: String) -> Result<()> {
    let record = HookRecord {
        event: HookEvent::Stop,
        window_id,
    };
    request(
        socket,
        Method::POST,
        "/v1/hooks",
        serde_json::to_vec(&record)?,
    )
    .await?;
    Ok(())
}

async fn request(socket: &Path, method: Method, path: &str, body: Vec<u8>) -> Result<Bytes> {
    timeout(
        Duration::from_secs(2),
        send_request(socket, method, path, body),
    )
    .await
    .map_err(|_| {
        format!(
            "The daemon at {} did not answer within 2 seconds.",
            socket.display()
        )
    })?
}

async fn send_request(socket: &Path, method: Method, path: &str, body: Vec<u8>) -> Result<Bytes> {
    let stream = UnixStream::connect(socket).await.map_err(|error| {
        format!(
            "Cannot connect to the daemon at {}: {error}. Start it with `ws daemon`.",
            socket.display()
        )
    })?;
    let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))?;
    let response = sender.send_request(request).await?;
    if !response.status().is_success() {
        return Err(format!("The daemon returned HTTP {}.", response.status()).into());
    }
    Ok(response.into_body().collect().await?.to_bytes())
}
