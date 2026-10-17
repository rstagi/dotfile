use std::{path::Path, time::Duration};

use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use tokio::{net::UnixStream, time::timeout};
use ws_core::PingResponse;

use crate::Result;

#[derive(Debug)]
struct ApiError {
    code: String,
    message: String,
    response: serde_json::Value,
}

pub async fn ping(socket: &Path) -> Result<PingResponse> {
    let body = request(socket, Method::GET, "/v1/ping", Vec::new()).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub async fn sessions(socket: &Path) -> Result<Vec<ws_core::registry::Session>> {
    let body = request(socket, Method::GET, "/v1/sessions", Vec::new()).await?;
    Ok(serde_json::from_slice(&body)?)
}

pub async fn session(socket: &Path, target: &str) -> Result<ws_core::registry::Session> {
    let path = format!("/v1/sessions/{}", encode(target));
    let body = request(socket, Method::GET, &path, Vec::new())
        .await
        .map_err(|error| match error.downcast::<ApiError>() {
            Ok(mut error) => {
                if error.code == "session_not_found" {
                    error.message = format!("no such session: {target}");
                }
                error as Box<dyn std::error::Error + Send + Sync>
            }
            Err(error) => error,
        })?;
    Ok(serde_json::from_slice(&body)?)
}

pub async fn rename(socket: &Path, id: &str, name: &str) -> Result<ws_core::registry::Session> {
    let path = format!("/v1/sessions/{}/name", encode(id));
    let body = request(
        socket,
        Method::POST,
        &path,
        serde_json::to_vec(&serde_json::json!({"name": name, "parent_session_id": id}))?,
    )
    .await?;
    Ok(serde_json::from_slice(&body)?)
}

pub(crate) fn error_json(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> serde_json::Value {
    error
        .downcast_ref::<ApiError>()
        .map(|error| error.response.clone())
        .unwrap_or_else(|| serde_json::json!({"error":{"message":error.to_string()}}))
}

pub fn is_missing(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|error| error.code == "session_not_found")
}

pub(crate) async fn request(
    socket: &Path,
    method: Method,
    path: &str,
    body: Vec<u8>,
) -> Result<Bytes> {
    if method == Method::POST
        && path.starts_with("/v1/sessions/")
        && (path.ends_with("/stop") || path.ends_with("/wait"))
    {
        return send_request(socket, method, path, body).await;
    }
    let seconds = if path.starts_with("/v1/workspaces") {
        120
    } else {
        2
    };
    timeout(
        Duration::from_secs(seconds),
        send_request(socket, method, path, body),
    )
    .await
    .map_err(|_| {
        format!(
            "The daemon at {} did not answer within {seconds} seconds.",
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
    let status = response.status();
    let body = response.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        return Err(Box::new(ApiError {
            response: error.clone(),
            code: error["error"]["code"].as_str().unwrap_or("").into(),
            message: error["error"]["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("The daemon returned HTTP {status}.")),
        }));
    }
    Ok(body)
}

pub(crate) fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
