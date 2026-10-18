//! Timer and phase-boundary collection through the session log writer.

use super::{Clock, Reader, binding::Binding, codex::CodexReader};
use crate::{api, sessions::Sessions};
use std::{
    collections::HashMap,
    fs, io,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use ws_core::ids::SessionId;

const PERIOD: Duration = Duration::from_secs(60);
type Store = Arc<Mutex<Sessions>>;
type ReadingKey = (SessionId, String, String, Option<String>, String, String);

#[derive(Clone, Copy, Debug)]
pub enum Trigger {
    Timer,
    Boundary(SessionId),
}

pub struct Requests {
    sender: mpsc::UnboundedSender<Trigger>,
    receiver: Option<mpsc::UnboundedReceiver<Trigger>>,
}

pub struct Collector {
    readers: HashMap<SessionId, Box<dyn Reader>>,
    last_rollout: HashMap<ReadingKey, serde_json::Value>,
    clock: Clock,
}

pub(crate) struct Service {
    timer: JoinHandle<()>,
    worker: JoinHandle<io::Result<()>>,
}

impl Collector {
    /// Collects after each supplied tick or boundary. Reader I/O never holds the session lock.
    pub async fn run(
        mut self,
        store: Store,
        mut requests: mpsc::UnboundedReceiver<Trigger>,
        mut shutdown: watch::Receiver<bool>,
    ) -> io::Result<()> {
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            let trigger = tokio::select! {
                biased;
                _ = shutdown.changed() => return Ok(()),
                trigger = requests.recv() => match trigger { Some(trigger) => trigger, None => return Ok(()) },
            };
            tokio::select! {
                biased;
                _ = shutdown.changed() => return Ok(()),
                result = self.collect(store.clone(), trigger) => {
                    if let Err(error) = result { eprintln!("Cannot collect runner quota: {error}"); }
                }
            }
        }
    }

    pub fn new(clock: Clock) -> Self {
        Self {
            readers: HashMap::new(),
            last_rollout: HashMap::new(),
            clock,
        }
    }

    pub fn register(&mut self, session_id: SessionId, reader: Box<dyn Reader>) {
        self.readers.insert(session_id, reader);
    }

    async fn collect(&mut self, store: Store, trigger: Trigger) -> io::Result<()> {
        let (state, sessions) = api::call(store.clone(), |sessions| {
            Ok((sessions.state.clone(), sessions.registry.sessions()?))
        })
        .await
        .map_err(|error| io::Error::other(error.0))?;
        let mut terminal = Vec::new();
        for session in sessions {
            let ended = ["finished", "failed"].contains(&session.state.as_str());
            let requested = matches!(trigger, Trigger::Boundary(id) if id == session.id);
            if ended {
                terminal.push(session.id);
            }
            if self.readers.contains_key(&session.id) || (ended && !requested) {
                continue;
            }
            let path = state
                .join("sessions")
                .join(session.id.to_string())
                .join("quota-binding.json");
            let binding = fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Binding>(&bytes).ok());
            if let Some(binding) = binding.filter(|binding| binding.runner == "codex") {
                self.register(
                    session.id,
                    Box::new(CodexReader::new(binding, state.clone(), session.id)),
                );
            }
        }
        for (id, reader) in &mut self.readers {
            let readings = match reader.read(self.clock.clone()).await {
                Ok(readings) => readings,
                Err(error) => {
                    eprintln!("Cannot read runner quota for {id}: {error}");
                    continue;
                }
            };
            for reading in readings {
                let body = reading.event_body();
                let key = (
                    *id,
                    reading.runner,
                    reading.account_id,
                    reading.bucket,
                    reading.window,
                    reading.source_name.clone(),
                );
                let mut snapshot = body.clone();
                snapshot.as_object_mut().unwrap().remove("received_at_ms");
                if reading.source_name == "rollout"
                    && self.last_rollout.get(&key) == Some(&snapshot)
                {
                    continue;
                }
                let session_id = *id;
                api::call(store.clone(), move |sessions| {
                    sessions.append(session_id, "runner.usage.updated", body)
                })
                .await
                .map_err(|error| io::Error::other(error.0))?;
                if reading.source_name == "rollout" {
                    self.last_rollout.insert(key, snapshot);
                }
            }
        }
        for id in terminal {
            self.readers.remove(&id);
            self.last_rollout.retain(|key, _| key.0 != id);
        }
        Ok(())
    }
}

impl Requests {
    pub fn sender(&self) -> mpsc::UnboundedSender<Trigger> {
        self.sender.clone()
    }

    pub fn take_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<Trigger>> {
        self.receiver.take()
    }

    pub(crate) fn observe(&self, id: SessionId, event_type: &str) {
        if matches!(
            event_type,
            "phase.started"
                | "phase.finished"
                | "phase.spawned"
                | "session.running"
                | "process.exited"
                | "exit"
                | "done"
        ) {
            let _ = self.sender.send(Trigger::Boundary(id));
        }
    }
}

impl Default for Requests {
    fn default() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: Some(receiver),
        }
    }
}

impl Service {
    pub(crate) fn start(store: Store) -> io::Result<Self> {
        Self::start_with_collector(store, Collector::new(Arc::new(now_ms)))
    }

    pub(crate) fn start_with_collector(store: Store, collector: Collector) -> io::Result<Self> {
        let (requests, sender, mut shutdown) = {
            let mut sessions = store.lock().map_err(|_| {
                io::Error::other("Cannot lock the session store for quota collection.")
            })?;
            (
                sessions
                    .quota_requests
                    .take_receiver()
                    .ok_or_else(|| io::Error::other("The quota collector already runs."))?,
                sessions.quota_requests.sender(),
                sessions.shutdown.subscribe(),
            )
        };
        let worker = tokio::spawn(collector.run(store, requests, shutdown.clone()));
        let timer = tokio::spawn(async move {
            let mut interval = tokio::time::interval(PERIOD);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => return,
                    _ = interval.tick() => { if sender.send(Trigger::Timer).is_err() { return; } }
                }
            }
        });
        Ok(Self { timer, worker })
    }

    pub(crate) async fn shutdown(mut self) {
        self.timer.abort();
        self.worker.abort();
        let _ = (&mut self.timer).await;
        let _ = (&mut self.worker).await;
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.timer.abort();
        self.worker.abort();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
