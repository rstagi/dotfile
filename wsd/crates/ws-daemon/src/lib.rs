//! The local WS daemon runtime.

use std::{
    fs::{self, File, TryLockError},
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{Json, Router, extract::State, routing::get};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};
use ws_core::{PingResponse, VERSION};

pub mod agents;
pub mod api;
pub mod bus;
pub mod edges;
pub mod hooks;
pub mod import;
pub mod notify;
pub mod phases;
pub mod sessions;
pub mod supervisor;
pub mod tmux;
pub mod triggers;
pub mod wait;
pub mod workspaces;

#[derive(Clone)]
struct AppState {
    started: Instant,
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
    let store = Arc::new(Mutex::new(sessions));
    supervisor::adopt(store.clone())
        .await
        .map_err(|error| io::Error::other(error.0))?;
    hooks::replay(store.clone(), &state).await?;
    api::call(store.clone(), |sessions| sessions.evaluate_triggers())
        .await
        .map_err(|error| io::Error::other(error.0))?;
    let delivery = bus::delivery::start(store.clone());
    let app = Router::new()
        .route("/v1/ping", get(ping))
        .with_state(AppState {
            started: Instant::now(),
        })
        .merge(api::router_with_store(store.clone()));
    let shutdown_store = store.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
            let _ = api::call(shutdown_store, |sessions| {
                sessions.shutdown.send_replace(true);
                Ok(())
            })
            .await;
        })
        .await;
    delivery.shutdown().await;
    result
}

fn lock_socket(socket: &Path) -> io::Result<File> {
    let mut path = socket.as_os_str().to_owned();
    path.push(".lock");
    // Keep this file after shutdown so every contender locks the same inode.
    let lock = ws_core::lockfile::open_lock_file(Path::new(&path))?;
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
