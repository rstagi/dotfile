//! Pushes the export projection of the session logs to the hub.
//!
//! The task runs only with a `[hub]` config section. It never blocks local work: it reads the
//! log files and talks to the hub on its own task, and it retries with a backoff.

pub mod export;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::task::JoinHandle;
use ws_core::{
    config::{Config, HubConfig},
    device::Device,
    ids::{SessionId, Ulid},
    log::SessionLog,
};
use ws_hub::{
    client::{HubClient, HubConnection},
    sync::{MAX_BATCH_ENTRIES, SessionBatch, SyncBatch},
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The time between two scans of the logs. A new entry reaches the hub within 5 seconds.
const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Stops the sync task when it is dropped.
pub struct SyncTask(JoinHandle<()>);

/// Starts the sync task when `config.toml` has a `[hub]` section.
pub fn start(state: &Path) -> Option<SyncTask> {
    let hub = match Config::load(state) {
        Ok(config) => config.hub?,
        Err(error) => {
            eprintln!("ws: hub sync is off: {error}");
            return None;
        }
    };
    Some(SyncTask(tokio::spawn(run(state.to_owned(), hub))))
}

impl Drop for SyncTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Per-session progress of this process.
#[derive(Default)]
struct Progress {
    /// The highest sequence number that the hub holds. Known after the first cursor read.
    pushed: u64,
    /// The byte offset in the log of the next unread entry.
    offset: u64,
}

struct Syncer {
    state: PathBuf,
    client: HubClient,
    device_id: Ulid,
    sessions: Option<HashMap<SessionId, Progress>>,
}

async fn run(state: PathBuf, hub: HubConfig) {
    let mut backoff = Duration::from_secs(1);
    let mut warned = false;
    let mut syncer: Option<Syncer> = None;
    loop {
        let result = match &mut syncer {
            Some(syncer) => syncer.cycle().await,
            None => match Syncer::new(&state, &hub) {
                Ok(new) => syncer.insert(new).cycle().await,
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(()) => {
                warned = false;
                backoff = Duration::from_secs(1);
                tokio::time::sleep(SCAN_INTERVAL).await;
            }
            Err(error) => {
                if !warned {
                    eprintln!("ws: hub sync cannot reach the hub: {error}. Retrying.");
                    warned = true;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

impl Syncer {
    fn new(state: &Path, hub: &HubConfig) -> Result<Self, Error> {
        Ok(Self {
            client: HubClient::new(&hub.url, &hub.cert, &hub.key, &hub.ca)?,
            device_id: Device::load(state)?.device_id,
            state: state.to_owned(),
            sessions: None,
        })
    }

    /// Pushes the new entries of each session. The first cycle reads the hub cursors.
    async fn cycle(&mut self) -> Result<(), Error> {
        let mut connection = self.client.connect().await?;
        let mut sessions = match self.sessions.take() {
            Some(sessions) => sessions,
            None => self.read_cursors(&mut connection).await?,
        };
        let mut result = Ok(());
        for id in self.session_ids() {
            result = self
                .push_session(&mut connection, id, sessions.entry(id).or_default())
                .await;
            if result.is_err() {
                break;
            }
        }
        self.sessions = Some(sessions);
        result
    }

    async fn read_cursors(
        &self,
        connection: &mut HubConnection,
    ) -> Result<HashMap<SessionId, Progress>, Error> {
        let response = connection.get("/v1/sync/cursors").await?;
        if response.status != 200 {
            return Err(format!("The hub answered {} to the cursor read.", response.status).into());
        }
        let cursors: HashMap<String, u64> = serde_json::from_str(&response.body)?;
        Ok(cursors
            .into_iter()
            .filter_map(|(id, seq)| {
                Some((
                    id.parse().ok()?,
                    Progress {
                        pushed: seq,
                        offset: 0,
                    },
                ))
            })
            .collect())
    }

    fn session_ids(&self) -> Vec<SessionId> {
        let Ok(entries) = std::fs::read_dir(self.state.join("sessions")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect()
    }

    async fn push_session(
        &self,
        connection: &mut HubConnection,
        id: SessionId,
        progress: &mut Progress,
    ) -> Result<(), Error> {
        let log = SessionLog::new(&self.state, id, self.device_id);
        let offset = progress.offset;
        let tail = tokio::task::spawn_blocking(move || log.read_after(offset)).await??;
        let pending: Vec<_> = tail
            .entries
            .iter()
            .filter(|e| e.seq > progress.pushed)
            .collect();
        let Some(last) = pending.last() else {
            progress.offset = tail.offset;
            return Ok(());
        };
        let through_seq = last.seq;
        let exported: Vec<_> = pending
            .iter()
            .filter_map(|entry| export::project(entry))
            .collect();
        // Send at least one request when the batch has no exported entry, so the cursor moves.
        let mut chunks: Vec<_> = exported
            .chunks(MAX_BATCH_ENTRIES)
            .map(<[_]>::to_vec)
            .collect();
        if chunks.is_empty() {
            chunks.push(Vec::new());
        }
        let chunk_count = chunks.len();
        for (index, entries) in chunks.into_iter().enumerate() {
            let through_seq = if index + 1 == chunk_count {
                through_seq
            } else {
                entries.last().map_or(0, |entry| entry.seq)
            };
            let batch = SyncBatch {
                sessions: vec![SessionBatch {
                    session_id: id.to_string(),
                    through_seq,
                    entries,
                }],
            };
            let response = connection
                .post_json("/v1/sync/logs", serde_json::to_vec(&batch)?)
                .await?;
            if response.status != 200 {
                return Err(format!("The hub answered {} to the push.", response.status).into());
            }
        }
        progress.pushed = through_seq;
        progress.offset = tail.offset;
        Ok(())
    }
}
