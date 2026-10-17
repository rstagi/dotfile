//! Bounded reads of the output files that the supervisor records.

use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use ws_core::ids::SessionId;

use crate::{
    api::{self, ApiError, Store},
    sessions::Error,
};

pub const DEFAULT_MAX_BYTES: u32 = 8192;
pub const MAX_BYTES: u32 = 65536;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TailRequest {
    parent_session_id: Option<SessionId>,
    #[serde(default = "default_stream")]
    stream: String,
    #[serde(default = "default_max_bytes")]
    max_bytes: u32,
}

#[derive(Serialize)]
struct TailResponse {
    session_id: SessionId,
    stream: String,
    output: String,
    truncated: bool,
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/sessions/{id}/tail", get(tail))
}

async fn tail(
    State(store): State<Store>,
    Path(id): Path<String>,
    Query(request): Query<TailRequest>,
) -> Result<Json<TailResponse>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(request.parent_session_id, "session.tail", Some(&id))?;
            if !["stdout", "stderr"].contains(&request.stream.as_str())
                || !(1..=MAX_BYTES).contains(&request.max_bytes)
            {
                return Err(Error::InvalidRequest(
                    "Use stdout or stderr and a byte limit from 1 to 65536.",
                ));
            }
            let session = sessions.get_record(&id)?;
            let path = session.body[&request.stream]
                .as_str()
                .ok_or(Error::InvalidRequest(
                    "The session has no output file for this stream.",
                ))?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err(Error::InvalidRequest(
                    "The session output must be a regular file.",
                ));
            }
            let start = metadata.len().saturating_sub(u64::from(request.max_bytes));
            file.seek(SeekFrom::Start(start))?;
            let mut bytes = Vec::with_capacity(request.max_bytes as usize);
            file.take(u64::from(request.max_bytes))
                .read_to_end(&mut bytes)?;
            let output = String::from_utf8_lossy(&bytes);
            // Invalid bytes can expand into replacement characters. Keep the byte bound after decoding.
            let mut offset = output.len().saturating_sub(request.max_bytes as usize);
            while !output.is_char_boundary(offset) {
                offset += 1;
            }
            Ok(TailResponse {
                session_id: session.id,
                stream: request.stream,
                output: output[offset..].into(),
                truncated: start > 0 || offset > 0,
            })
        })
        .await?,
    ))
}

fn default_stream() -> String {
    "stdout".into()
}
fn default_max_bytes() -> u32 {
    DEFAULT_MAX_BYTES
}
