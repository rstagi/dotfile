//! The local WS daemon runtime.

use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

use axum::{Json, Router, extract::State, routing::get};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};
use ws_core::{PingResponse, VERSION};

pub mod tmux;

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
        .with_state(Instant::now());
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        })
        .await
}

async fn ping(State(started): State<Instant>) -> Json<PingResponse> {
    Json(PingResponse {
        version: VERSION.to_owned(),
        uptime_secs: started.elapsed().as_secs(),
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
