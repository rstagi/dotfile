//! The local WS daemon runtime.

use std::{
    collections::VecDeque,
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};
use ws_core::{HookRecord, PingResponse, VERSION};

pub mod tmux;

const HOOK_CAPACITY: usize = 256;
type ApiError = (StatusCode, &'static str);

#[derive(Clone)]
struct AppState {
    started: Instant,
    hooks: Arc<Mutex<VecDeque<HookRecord>>>,
}

struct SocketFile {
    path: PathBuf,
    device: u64,
    inode: u64,
}

pub async fn run(socket: &Path) -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    if let Some(parent) = socket.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    let metadata = fs::symlink_metadata(socket)?;
    let _socket_file = SocketFile {
        path: socket.to_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    let app = Router::new()
        .route("/v1/ping", get(ping))
        .route("/v1/hooks", get(hooks).post(record_hook))
        .with_state(AppState {
            started: Instant::now(),
            hooks: Arc::new(Mutex::new(VecDeque::new())),
        });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        })
        .await
}

async fn ping(State(state): State<AppState>) -> Json<PingResponse> {
    Json(PingResponse {
        version: VERSION.to_owned(),
        uptime_secs: state.started.elapsed().as_secs(),
    })
}

async fn hooks(State(state): State<AppState>) -> Result<Json<Vec<HookRecord>>, ApiError> {
    let hooks = state.hooks.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot read the hook store.",
        )
    })?;
    Ok(Json(hooks.iter().cloned().collect()))
}

async fn record_hook(
    State(state): State<AppState>,
    Json(record): Json<HookRecord>,
) -> Result<(StatusCode, Json<HookRecord>), ApiError> {
    if !record
        .window_id
        .strip_prefix('@')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err((StatusCode::BAD_REQUEST, "Use a tmux window id such as @1."));
    }
    let mut hooks = state.hooks.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot write the hook store.",
        )
    })?;
    if hooks.len() == HOOK_CAPACITY {
        hooks.pop_front();
    }
    hooks.push_back(record.clone());
    Ok((StatusCode::CREATED, Json(record)))
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
