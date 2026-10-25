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
    api::{self, ApiError, CallContext, Store},
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

#[derive(Serialize)]
struct OutputResponse {
    session_id: SessionId,
    output: String,
    truncated: bool,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/sessions/{id}/tail", get(tail))
        .route("/v1/sessions/{id}/output", get(output))
}

async fn output(
    State(store): State<Store>,
    Path(id): Path<String>,
    Query(context): Query<CallContext>,
) -> Result<Json<OutputResponse>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.record_call(context.parent_session_id, "session.output", Some(&id))?;
            let session = sessions.get_record(&id)?;
            let path = sessions
                .state
                .join("sessions")
                .join(session.id.to_string())
                .join("output.md");
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                .open(path)?;
            if !file.metadata()?.is_file() {
                return Err(Error::InvalidRequest(
                    "The session output must be a regular file.",
                ));
            }
            let mut bytes = Vec::with_capacity(MAX_BYTES as usize + 1);
            file.take(u64::from(MAX_BYTES) + 1)
                .read_to_end(&mut bytes)?;
            let truncated = bytes.len() > MAX_BYTES as usize;
            bytes.truncate(MAX_BYTES as usize);
            // Preserve complete UTF-8 characters at the byte limit.
            if truncated {
                if let Err(error) = std::str::from_utf8(&bytes) {
                    if error.error_len().is_none() {
                        bytes.truncate(error.valid_up_to());
                    }
                }
            }
            let mut output = String::from_utf8_lossy(&bytes).into_owned();
            let mut end = output.len().min(MAX_BYTES as usize);
            while !output.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = truncated || end < output.len();
            output.truncate(end);
            Ok(OutputResponse {
                session_id: session.id,
                output,
                truncated,
            })
        })
        .await?,
    ))
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
