use std::{
    fs::{self, OpenOptions, TryLockError},
    io,
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, body::Bytes, client::conn::http1};
use hyper_util::rt::TokioIo;
use tokio::{
    net::UnixStream,
    time::{sleep, timeout},
};
use ws_core::PingResponse;

use crate::Result;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

struct StartingDaemon(Option<Child>);

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
    // Hooks retain their bounded spool fallback. Notification dismissal is best effort.
    let autostart = !path.starts_with("/v1/hooks/") && path != "/v1/notifications/dismiss";
    let stream = connect(socket, autostart).await?;
    // These calls wait for a lifecycle operation or a daemon wait condition.
    // A client deadline can report failure after the daemon records a durable change.
    if method == Method::POST
        && (matches!(path, "/v1/agents/launch" | "/v1/messages")
            || path.starts_with("/v1/sessions/")
                && (path.ends_with("/stop") || path.ends_with("/wait")))
    {
        return send_request(stream, method, path, body).await;
    }
    // Store requests can wait behind lifecycle changes. Their deadline is a hang guard.
    let seconds = if path.starts_with("/v1/workspaces") || path == "/v1/phases/worktree" {
        120
    } else if path == "/v1/ping"
        || path.starts_with("/v1/hooks/")
        || path == "/v1/notifications/dismiss"
    {
        2
    } else {
        30
    };
    timeout(
        Duration::from_secs(seconds),
        send_request(stream, method, path, body),
    )
    .await
    .map_err(|_| {
        format!(
            "The daemon at {} did not answer within {seconds} seconds.",
            socket.display()
        )
    })?
}

async fn connect(socket: &Path, autostart: bool) -> Result<UnixStream> {
    match UnixStream::connect(socket).await {
        Ok(stream) => Ok(stream),
        Err(error) => {
            if autostart && error.kind() == io::ErrorKind::NotFound {
                return timeout(STARTUP_TIMEOUT, start_daemon(socket))
                    .await
                    .map_err(|_| {
                        format!(
                            "The daemon at {} did not start within 30 seconds.",
                            socket.display()
                        )
                    })?;
            }
            Err(connection_error(socket, error).into())
        }
    }
}

async fn start_daemon(socket: &Path) -> Result<UnixStream> {
    if let Some(parent) = socket.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut lock_path = socket.as_os_str().to_owned();
    lock_path.push(".start.lock");
    let lock = ws_core::lockfile::open_lock_file(Path::new(&lock_path))?;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                // The startup owner releases this lock after the daemon answers a ping.
                sleep(Duration::from_millis(20)).await;
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    if let Ok(stream) = UnixStream::connect(socket).await {
        return Ok(stream);
    }
    if launchd_owns_daemon().await {
        return Err(format!(
            "Cannot connect to the daemon at {}. launchd owns com.rstagi.wsd. Check the LaunchAgent.",
            socket.display()
        ).into());
    }
    let log_path = socket.with_extension("daemon.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)?;
    let child = Command::new(std::env::current_exe()?)
        .arg("daemon")
        .env("WS_SOCK", socket)
        .env_remove("WS_SESSION_ID")
        .env_remove("WS_SESSION")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let mut starting = StartingDaemon(Some(child));
    loop {
        if let Some(status) = starting.0.as_mut().unwrap().try_wait()? {
            return Err(format!(
                "The daemon exited with {status}. Read {}.",
                log_path.display()
            )
            .into());
        }
        if let Ok(probe) = UnixStream::connect(socket).await
            && send_request(probe, Method::GET, "/v1/ping", Vec::new())
                .await
                .is_ok()
            && let Ok(stream) = UnixStream::connect(socket).await
        {
            let mut child = starting.0.take().unwrap();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(stream);
        }
        sleep(Duration::from_millis(20)).await;
    }
}

async fn launchd_owns_daemon() -> bool {
    let Ok(uid) = tokio::process::Command::new("id")
        .arg("-u")
        .kill_on_drop(true)
        .output()
        .await
    else {
        return false;
    };
    if !uid.status.success() {
        return false;
    }
    let service = format!(
        "gui/{}/com.rstagi.wsd",
        String::from_utf8_lossy(&uid.stdout).trim()
    );
    tokio::process::Command::new("launchctl")
        .args(["print", &service])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|status| status.success())
}

fn connection_error(socket: &Path, error: io::Error) -> String {
    format!(
        "Cannot connect to the daemon at {}: {error}. Start it with `ws daemon`.",
        socket.display()
    )
}

async fn send_request(
    stream: UnixStream,
    method: Method,
    path: &str,
    body: Vec<u8>,
) -> Result<Bytes> {
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

impl Drop for StartingDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::UnixListener,
        sync::oneshot,
    };

    #[tokio::test]
    async fn store_requests_wait_for_completion_while_fast_requests_keep_their_deadline() {
        let mut failures = Vec::new();
        for (method, path, waits) in [
            (Method::POST, "/v1/agents/launch", true),
            (Method::POST, "/v1/messages", true),
            (Method::POST, "/v1/agents/run", true),
            (Method::POST, "/v1/agents/exit", true),
            (Method::GET, "/v1/sessions/target", true),
            (Method::GET, "/v1/ping", false),
            (Method::POST, "/v1/hooks/ingest", false),
            (Method::POST, "/v1/notifications/dismiss", false),
        ] {
            let home = tempfile::tempdir_in("/tmp").unwrap();
            let socket = home.path().join("daemon.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let (observed, received) = oneshot::channel();
            let (release, released) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let n = stream.read(&mut bytes).await.unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&bytes[..n]);
                }
                observed.send(()).unwrap();
                released.await.unwrap();
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await;
            });
            let client =
                tokio::spawn(async move { request(&socket, method, path, Vec::new()).await });
            received.await.unwrap();
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(3)).await;
            tokio::task::yield_now().await;
            tokio::time::resume();
            release.send(()).unwrap();
            let result = client.await.unwrap();
            server.await.unwrap();
            match result {
                Ok(body) if waits => assert_eq!(body.as_ref(), b"{}"),
                Ok(_) => failures.push(format!("{path} did not enforce its deadline.")),
                Err(error) if !waits => assert!(error.to_string().contains("within 2 seconds")),
                Err(error) => failures.push(format!("{path}: {error}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
