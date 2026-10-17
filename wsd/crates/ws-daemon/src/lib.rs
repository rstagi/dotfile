//! The local WS daemon runtime.

use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
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

pub mod api;
pub mod sessions;
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
    let _lock = lock_socket(socket)?;
    let listener = bind(socket)?;
    let metadata = fs::symlink_metadata(socket)?;
    let _socket_file = SocketFile {
        path: socket.to_owned(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    let state = match std::env::var_os("WS_STATE") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Set HOME or WS_STATE to locate the session state.",
            )
        })?)
        .join(".ws/state"),
    };
    let sessions = sessions::Sessions::open(&state, 1).map_err(io::Error::other)?;
    let app = Router::new()
        .route("/v1/ping", get(ping))
        .route("/v1/hooks", get(hooks).post(record_hook))
        .with_state(AppState {
            started: Instant::now(),
            hooks: Arc::new(Mutex::new(VecDeque::new())),
        })
        .merge(api::router(sessions));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        })
        .await
}

fn lock_socket(socket: &Path) -> io::Result<File> {
    let mut path = socket.as_os_str().to_owned();
    path.push(".lock");
    // Keep this file after shutdown so every contender locks the same inode.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "Another daemon holds the socket lock for {}.",
                socket.display()
            ),
        )),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Replaces stale socket files while the caller holds the daemon lock. Preserves other files.
fn bind(socket: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(socket) {
        Err(error)
            if error.kind() == io::ErrorKind::AddrInUse
                && fs::symlink_metadata(socket)
                    .is_ok_and(|metadata| metadata.file_type().is_socket()) =>
        {
            fs::remove_file(socket)?;
            UnixListener::bind(socket)
        }
        result => result,
    }
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
