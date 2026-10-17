//! Durable session lifecycle operations.

use std::{
    collections::HashMap,
    fmt,
    fs::{File, TryLockError},
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use ws_core::{
    device::Device,
    ids::{DeviceId, SessionId, Ulid},
    lockfile,
    log::{LogWriter, SessionLog},
    registry::{self, Registry, Session},
    state::{SessionState, TransitionError},
};

pub struct Sessions {
    pub(crate) trigger_events_pending: bool,
    pub(crate) triggers: std::collections::BTreeMap<Ulid, crate::triggers::Trigger>,
    pub(crate) state: PathBuf,
    pub(crate) device_id: DeviceId,
    pub(crate) registry: Registry,
    needs_rebuild: bool,
    writers: HashMap<SessionId, LogWriter>,
    pub(crate) headless_hooks: HashMap<SessionId, crate::hooks::HookState>,
    pub(crate) changes: tokio::sync::watch::Sender<()>,
    pub(crate) shutdown: tokio::sync::watch::Sender<bool>,
    _writer_lock: File,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSession {
    pub name: Option<String>,
    pub workspace_id: Option<String>,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Registry(registry::Error),
    InvalidRequest(&'static str),
    Transition(TransitionError),
    NameTaken(String),
    DependencyCycle(Vec<SessionId>),
    PendingMessages,
    WaitTimeout,
    ShuttingDown,
    Json(serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Sessions {
    /// Opens the local index and restores it from the session logs.
    pub fn open(state: &Path, alias_floor: i64) -> Result<Self> {
        let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into());
        let device = Device::init(state, &hostname)?;
        let writer_lock = lockfile::open_lock_file(&state.join("sessions.lock"))?;
        match writer_lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "Another lifecycle writer holds the session state lock.",
                )
                .into());
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let mut registry = Registry::open(&state.join("registry.sqlite"), alias_floor)?;
        registry.rebuild(state)?;
        let mut sessions = Self {
            triggers: Default::default(),
            trigger_events_pending: false,
            state: state.into(),
            device_id: device.device_id,
            registry,
            needs_rebuild: false,
            writers: HashMap::new(),
            headless_hooks: HashMap::new(),
            changes: tokio::sync::watch::channel(()).0,
            shutdown: tokio::sync::watch::channel(false).0,
            _writer_lock: writer_lock,
        };
        sessions.restore_triggers()?;
        Ok(sessions)
    }

    pub fn create(&mut self, request: CreateSession) -> Result<Session> {
        let id = Ulid::new();
        self.record_call(
            request.parent_session_id,
            "session.create",
            Some(&id.to_string()),
        )?;
        if request
            .name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(Error::InvalidRequest("The session name is empty."));
        }
        if request
            .workspace_id
            .as_ref()
            .is_some_and(|workspace| workspace.trim().is_empty())
        {
            return Err(Error::InvalidRequest("The workspace id is empty."));
        }
        let alias = self.registry.next_alias()?;
        let body = json!({"alias": alias, "name": request.name, "workspace_id": request.workspace_id,
            "parent_session_id": request.parent_session_id});
        self.append(id, "session.created", body)?;
        self.get_record(&id.to_string())
    }

    pub fn list(&mut self, parent_session_id: Option<SessionId>) -> Result<Vec<Session>> {
        self.record_call(parent_session_id, "session.list", None)?;
        Ok(self.registry.sessions()?)
    }

    pub fn get(&mut self, input: &str, parent_session_id: Option<SessionId>) -> Result<Session> {
        self.record_call(parent_session_id, "session.get", Some(input))?;
        self.get_record(input)
    }

    pub fn rename(
        &mut self,
        input: &str,
        name: &str,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        self.record_call(parent_session_id, "session.rename", Some(input))?;
        let session = self.get_record(input)?;
        if name == "me"
            || !name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic())
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        {
            return Err(Error::InvalidRequest(
                "name: use a letter followed by letters, digits, dots, underscores or hyphens (me is reserved)",
            ));
        }
        if self
            .registry
            .sessions()?
            .iter()
            .any(|other| other.id != session.id && other.name.as_deref() == Some(name))
        {
            return Err(Error::NameTaken(name.into()));
        }
        if session.name.as_deref() != Some(name) {
            self.append(
                session.id,
                "session.renamed",
                json!({"name": name, "parent_session_id": parent_session_id}),
            )?;
        }
        self.get_record(&session.id.to_string())
    }

    /// Commits a legal transition before updating the query projection.
    pub fn transition(
        &mut self,
        input: &str,
        to: SessionState,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        self.record_call(parent_session_id, "session.transition", Some(input))?;
        self.change_state(input, to, parent_session_id)
    }

    pub fn stop(&mut self, input: &str, parent_session_id: Option<SessionId>) -> Result<Session> {
        self.record_call(parent_session_id, "session.stop", Some(input))?;
        self.change_state(input, SessionState::Finished, parent_session_id)
    }

    pub(crate) fn get_record(&self, input: &str) -> Result<Session> {
        let id = self.registry.resolve(input)?;
        self.registry
            .sessions()?
            .into_iter()
            .find(|session| session.id == id)
            .ok_or_else(|| Error::Registry(registry::Error::NotFound(input.into())))
    }

    /// Anonymous calls have no caller log. Attributed calls append even when the target is invalid.
    pub(crate) fn record_call(
        &mut self,
        parent_session_id: Option<SessionId>,
        operation: &str,
        target: Option<&str>,
    ) -> Result<()> {
        // A failed append or index update can leave a durable record. Recover before another call.
        if self.needs_rebuild {
            self.registry.rebuild(&self.state)?;
            self.restore_triggers()?;
            self.needs_rebuild = false;
        }
        if let Some(parent) = parent_session_id {
            let session = self.get_record(&parent.to_string())?;
            self.append(
                session.id,
                "api.called",
                json!({"parent_session_id": parent, "operation": operation, "target": target}),
            )?;
        }
        Ok(())
    }

    pub(crate) fn append(
        &mut self,
        id: SessionId,
        event_type: &str,
        body: serde_json::Value,
    ) -> Result<()> {
        self.append_entry(id, event_type, body).map(|_| ())
    }

    pub(crate) fn append_entry(
        &mut self,
        id: SessionId,
        event_type: &str,
        body: serde_json::Value,
    ) -> Result<ws_core::log::LogEntry> {
        let writer = self
            .writers
            .entry(id)
            .or_insert_with(|| SessionLog::new(&self.state, id, self.device_id).writer());
        self.needs_rebuild = true;
        let entry = writer.append_current(event_type, body)?;
        self.registry.apply(&entry)?;
        self.needs_rebuild = false;
        self.apply_trigger_entry(&entry);
        self.changes.send_replace(());
        self.trigger_events_pending |= ["done", "exit"].contains(&event_type);
        Ok(entry)
    }

    fn change_state(
        &mut self,
        input: &str,
        to: SessionState,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        let session = self.get_record(input)?;
        let from: SessionState =
            serde_json::from_value(json!(session.state)).map_err(Error::Json)?;
        from.transition(to).map_err(Error::Transition)?;
        self.append(
            session.id,
            to.event_type(),
            json!({"parent_session_id": parent_session_id}),
        )?;
        self.get_record(input)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShuttingDown => {
                formatter.write_str("The daemon stopped before the wait condition held.")
            }
            Self::WaitTimeout => {
                formatter.write_str("The wait condition did not hold before the timeout.")
            }
            Self::PendingMessages => formatter
                .write_str("The session has pending messages. Process them before completion."),
            Self::DependencyCycle(cycle) => write!(
                formatter,
                "Dependency cycle: {}.",
                cycle
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::Io(error) => error.fmt(formatter),
            Self::Registry(error) => error.fmt(formatter),
            Self::InvalidRequest(message) => formatter.write_str(message),
            Self::Transition(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::NameTaken(name) => write!(formatter, "name already in use: {name}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<registry::Error> for Error {
    fn from(error: registry::Error) -> Self {
        Self::Registry(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permits_only_one_lifecycle_writer_for_a_state_directory() {
        let sandbox = tempfile::tempdir().unwrap();
        let first = Sessions::open(sandbox.path(), 1).unwrap();
        assert!(
            matches!(Sessions::open(sandbox.path(), 1), Err(Error::Io(error)) if error.kind() == io::ErrorKind::AddrInUse)
        );
        drop(first);
        assert!(Sessions::open(sandbox.path(), 1).is_ok());
    }
}
