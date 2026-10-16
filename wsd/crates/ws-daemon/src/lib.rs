//! The local WS daemon runtime.

use std::{io, path::Path, time::Instant};

use axum::{Json, Router, extract::State, routing::get};
use tokio::net::UnixListener;
use ws_core::{PingResponse, VERSION};

pub async fn run(socket: &Path) -> io::Result<()> {
    let listener = UnixListener::bind(socket)?;
    let app = Router::new()
        .route("/v1/ping", get(ping))
        .with_state(Instant::now());
    axum::serve(listener, app).await
}

async fn ping(State(started): State<Instant>) -> Json<PingResponse> {
    Json(PingResponse {
        version: VERSION.to_owned(),
        uptime_secs: started.elapsed().as_secs(),
    })
}
