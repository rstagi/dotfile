use std::{path::Path, time::Duration};

use http_body_util::{BodyExt, Empty};
use hyper::{Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use tokio::{net::UnixStream, time::timeout};
use ws_core::PingResponse;

use crate::Result;

pub async fn ping(socket: &Path) -> Result<PingResponse> {
    timeout(Duration::from_secs(2), request_ping(socket))
        .await
        .map_err(|_| {
            format!(
                "The daemon at {} did not answer within 2 seconds.",
                socket.display()
            )
        })?
}

async fn request_ping(socket: &Path) -> Result<PingResponse> {
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
        .uri("/v1/ping")
        .header("Host", "localhost")
        .body(Empty::<Bytes>::new())?;
    let response = sender.send_request(request).await?;
    if !response.status().is_success() {
        return Err(format!("The daemon returned HTTP {}.", response.status()).into());
    }
    let body = response.into_body().collect().await?.to_bytes();
    Ok(serde_json::from_slice(&body)?)
}
