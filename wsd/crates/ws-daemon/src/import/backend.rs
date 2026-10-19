//! Coordinates legacy import, delivery and alias allocation.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Result, Sessions},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::{self, File, TryLockError},
    io,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Request {
    backend: String,
    home: PathBuf,
    root: PathBuf,
}

pub fn home() -> io::Result<PathBuf> {
    std::env::var_os("WS_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".ws")))
        .ok_or_else(|| io::Error::other("Set HOME or WS_HOME to locate the backend selector."))
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/backend", post(prepare))
}

async fn prepare(
    State(store): State<Store>,
    Json(request): Json<Request>,
) -> std::result::Result<Json<Value>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            let home =
                sessions
                    .legacy_home
                    .clone()
                    .ok_or(crate::sessions::Error::InvalidRequest(
                        "The daemon has no zsh state directory.",
                    ))?;
            if home.canonicalize()? != request.home.canonicalize()? {
                return Err(crate::sessions::Error::InvalidRequest(
                    "The CLI and daemon use different WS_HOME directories.",
                ));
            }
            match request.backend.as_str() {
                "rust" => {
                    super::incremental(sessions, &home, &request.root)?;
                    Ok(json!({}))
                }
                _ => Err(crate::sessions::Error::InvalidRequest(
                    "Only the rust backend is available.",
                )),
            }
        })
        .await?,
    ))
}

impl Sessions {
    /// Opens a service that shares aliases with this zsh state directory.
    pub fn open_with_home(state: &Path, alias_floor: i64, home: &Path) -> Result<Self> {
        fs::create_dir_all(home.join("sessions"))?;
        let mut sessions = Self::open(state, alias_floor)?;
        sessions.legacy_home = Some(home.into());
        reserve_counter(home, sessions.registry.next_alias()? - 1, false)?;
        Ok(sessions)
    }

    pub(crate) fn next_alias(&self) -> Result<i64> {
        let floor = self.registry.next_alias()?;
        let Some(home) = &self.legacy_home else {
            return Ok(floor);
        };
        Ok(reserve_counter(home, floor, true)?)
    }

    /// Returns a shared import lock. An import in progress suppresses injection.
    pub(crate) fn delivery_guard(&self) -> io::Result<Option<File>> {
        let Some(home) = &self.legacy_home else {
            return Ok(Some(ws_core::lockfile::open_lock_file(
                &self.state.join("backend.lock"),
            )?));
        };
        delivery_guard(home)
    }
}

fn reserve_counter(home: &Path, floor: i64, allocate: bool) -> io::Result<i64> {
    let directory = home.join("sessions");
    let lock = ws_core::lockfile::open_lock_file(&directory.join("counter.lock"))?;
    let request = libc::flock {
        l_start: 0,
        l_len: 0,
        l_pid: 0,
        l_type: libc::F_WRLCK as _,
        l_whence: libc::SEEK_SET as _,
    };
    loop {
        // SAFETY: the open descriptor and initialized request remain valid during fcntl.
        if unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_SETLKW, &request) } != -1 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    let counter = match fs::read_to_string(directory.join("counter")) {
        Ok(value) => value.trim().parse::<i64>().map_err(io::Error::other)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let alias = counter
        .checked_add(i64::from(allocate))
        .map(|n| n.max(floor))
        .filter(|n| counter >= 0 && *n >= 0 && *n < (1 << 53))
        .ok_or_else(|| io::Error::other("The zsh session counter is invalid."))?;
    super::atomic_write(&directory.join("counter"), format!("{alias}\n").as_bytes())?;
    Ok(alias)
}

/// Holds a shared import lock until the caller finishes physical delivery.
pub fn delivery_guard(home: &Path) -> io::Result<Option<File>> {
    let lock = ws_core::lockfile::open_lock_file(&home.join("backend.lock"))?;
    match lock.try_lock_shared() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    Ok(Some(lock))
}
