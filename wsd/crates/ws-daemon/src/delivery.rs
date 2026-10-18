//! Durable Stop replies and idle paste batches share the recipient inbox.

use crate::{
    api::{self, ApiError, Store},
    bus::Message,
    hooks::{Callback, StopDecision},
    sessions::{Error, Result, Sessions},
};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use ws_core::{
    ids::{EventId, SessionId, Ulid},
    log::SessionLog,
};

const RETRY_INTERVAL: Duration = Duration::from_millis(250);
// Separate the TUI paste handler from Enter, as ws_bus_paste does.
const SUBMIT_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub session_id: SessionId,
    pub callback_id: Ulid,
}

#[derive(Deserialize, Serialize)]
struct Batch {
    callback_id: Ulid,
    message_ids: Vec<EventId>,
    reason: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Server {
    socket: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Paste {
    batch_id: Ulid,
    #[serde(skip)]
    submitted: bool,
    message_ids: Vec<EventId>,
    watermark_us: u64,
    deadline_us: u64,
    window: String,
    pane: String,
    server: Server,
}

type Clock = std::sync::Arc<dyn Fn() -> io::Result<u64> + Send + Sync>;

pub(crate) struct Worker(tokio::task::JoinHandle<()>);

/// Starts one retry worker. The daemon owns and cancels this worker.
pub(crate) fn start(store: Store) -> Worker {
    let socket = std::env::var("WS_TMUX_SOCKET")
        .ok()
        .filter(|s| !s.is_empty());
    let path = std::env::var("TMUX")
        .ok()
        .and_then(|s| s.split(',').next().map(str::to_owned));
    let server = Server {
        socket: socket.or_else(|| path.is_none().then(|| "default".into())),
        path,
    };
    let timeout = std::env::var("WS_PASTE_ACK_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .and_then(|s| Duration::try_from_secs_f64(s).ok())
        .filter(|d| !d.is_zero())
        .unwrap_or(Duration::from_secs(10));
    Worker::spawn(store, server, timeout, RETRY_INTERVAL)
}

impl Worker {
    pub(crate) async fn shutdown(mut self) {
        self.0.abort();
        let _ = (&mut self.0).await;
    }

    fn spawn(store: Store, server: Server, timeout: Duration, interval: Duration) -> Self {
        Self::with_clock(
            store,
            server,
            timeout,
            interval,
            std::sync::Arc::new(crate::hooks::receipt_time_us),
        )
    }

    fn with_clock(
        store: Store,
        server: Server,
        timeout: Duration,
        interval: Duration,
        clock: Clock,
    ) -> Self {
        Self(tokio::spawn(async move {
            loop {
                if let Err(error) = retry(store.clone(), &server, timeout, &clock).await {
                    eprintln!("Cannot retry message delivery: {error}");
                }
                tokio::time::sleep(interval).await;
            }
        }))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn retry(store: Store, server: &Server, timeout: Duration, clock: &Clock) -> Result<()> {
    // Hooks and pastes must use the same lock before they examine the inbox or window.
    let guard = crate::hooks::INGEST_LOCK.lock().await;
    let now = clock()?;
    let targets = api::call(store.clone(), move |sessions| {
        let pending: HashSet<_> = sessions
            .registry
            .messages()?
            .into_iter()
            .filter(|m| m.delivered_at_ms.is_none() && m.read_at_ms.is_none())
            .map(|m| m.recipient)
            .collect();
        let local: Vec<_> = sessions
            .registry
            .sessions()?
            .into_iter()
            .filter(|s| s.device_id == sessions.device_id && pending.contains(&s.id))
            .collect();
        let mut available = HashSet::new();
        for session in local {
            let id = session.id;
            if let Some(paste) = sessions.active_paste(id)? {
                if now >= paste.deadline_us {
                    sessions.requeue_paste(id, paste.batch_id)?;
                }
            } else if !matches!(session.state.as_str(), "finished" | "failed") {
                available.insert(id);
            }
        }
        let mut targets = Vec::new();
        for workspace in sessions.registry.workspaces()? {
            for window in workspace.body["windows"].as_array().into_iter().flatten() {
                let Some(id) = window["session_id"]
                    .as_str()
                    .and_then(|s| s.parse::<SessionId>().ok())
                else {
                    continue;
                };
                if available.contains(&id)
                    && let Some(window) = window["window_id"]
                        .as_str()
                        .filter(|s| valid_target(s, '@'))
                {
                    targets.push((id, window.to_owned()));
                }
            }
        }
        Ok(targets)
    })
    .await
    .map_err(|e| e.0)?;
    drop(guard);
    for (id, window) in targets {
        let _guard = crate::hooks::INGEST_LOCK.lock().await;
        // A missing window must not prevent another session from receiving its messages.
        if let Err(error) = paste_idle(store.clone(), server, id, &window, timeout, clock).await {
            eprintln!("Cannot paste messages for {id}: {error}");
        }
    }
    Ok(())
}

async fn paste_idle(
    store: Store,
    server: &Server,
    id: SessionId,
    window: &str,
    timeout: Duration,
    clock: &Clock,
) -> Result<()> {
    let Some(_backend) = api::call(store.clone(), |sessions| Ok(sessions.delivery_guard()?))
        .await
        .map_err(|e| e.0)?
    else {
        return Ok(());
    };
    let snapshot = server
        .run(&[
            "display-message",
            "-p",
            "-t",
            window,
            "#{@ws_session}|#{@ws_state}|#{@ws_main}|#{@ws_pane}|#{socket_path}",
        ])
        .await?;
    let fields: Vec<_> = snapshot.split('|').collect();
    if fields.len() != 5
        || fields[0] != id.to_string()
        || fields[1] != "idle"
        || (!fields[2].is_empty() && fields[2] != "idle")
    {
        return Ok(());
    }
    let panes = server
        .run(&["list-panes", "-t", window, "-F", "#{pane_id}"])
        .await?;
    let pane = if fields[3].is_empty() {
        if panes.lines().count() != 1 {
            return Ok(());
        }
        panes.as_str()
    } else {
        fields[3]
    };
    if !valid_target(pane, '%') || !panes.lines().any(|p| p == pane) {
        return Ok(());
    }
    // A new agent window can still contain its launch shell. Wait for TUI input readiness.
    if fields[2].is_empty()
        && server
            .run(&["display-message", "-p", "-t", pane, "#{bracket_paste_flag}"])
            .await?
            != "1"
    {
        return Ok(());
    }
    let pending = api::call(store.clone(), move |sessions| {
        if sessions.active_paste(id)?.is_some() {
            return Ok(None);
        }
        let settled: HashSet<_> = sessions
            .registry
            .messages()?
            .into_iter()
            .filter(|m| m.delivered_at_ms.is_some() || m.read_at_ms.is_some())
            .map(|m| m.id)
            .collect();
        Ok(Some(
            sessions
                .inbox(id)?
                .into_iter()
                .filter(|m| !settled.contains(&m.event_id))
                .collect::<Vec<_>>(),
        ))
    })
    .await
    .map_err(|e| e.0)?;
    let Some(messages) = pending.filter(|m| !m.is_empty()) else {
        return Ok(());
    };
    let watermark_us = clock()?;
    let paste = Paste {
        batch_id: Ulid::new(),
        submitted: false,
        message_ids: messages.iter().map(|m| m.event_id).collect(),
        watermark_us,
        deadline_us: watermark_us.saturating_add(
            timeout
                .as_micros()
                .saturating_add(SUBMIT_DELAY.as_micros())
                .min(u64::MAX as u128) as u64,
        ),
        window: window.into(),
        pane: pane.into(),
        server: Server {
            socket: server.socket.clone(),
            path: Some(fields[4].into()),
        },
    };
    let batch_id = paste.batch_id;
    let guard = ownership(&paste, id);
    let record = serde_json::to_value(&paste).map_err(Error::Json)?;
    api::call(store.clone(), move |sessions| {
        sessions.append(id, "paste.started", record)
    })
    .await
    .map_err(|e| e.0)?;
    let buffer = format!("ws-paste-{batch_id}");
    let text = messages.iter().map(render).collect::<Vec<_>>().join("\n\n");
    let result = submit(server, &paste, &guard, &buffer, &text).await;
    if result.is_ok() {
        api::call(store.clone(), move |sessions| {
            sessions.append(id, "paste.submitted", json!({"batch_id":batch_id}))
        })
        .await
        .map_err(|e| e.0)?;
    } else {
        let _ = server.run(&["delete-buffer", "-b", &buffer]).await;
        api::call(store, move |sessions| sessions.requeue_paste(id, batch_id))
            .await
            .map_err(|e| e.0)?;
    }
    result.map_err(Into::into)
}

async fn submit(
    server: &Server,
    paste: &Paste,
    guard: &str,
    buffer: &str,
    text: &str,
) -> io::Result<()> {
    let Paste {
        pane,
        window,
        watermark_us,
        ..
    } = paste;
    let idle = format!("#{{&&:{guard},#{{==:#{{@ws_state}},idle}}}}");
    let reserve = format!(
        "set-option -w -t {window} @ws_seq {watermark_us}; \
         set-option -wu -t {window} @ws_turn; display-message -p reserved"
    );
    let reserved = server
        .run(&["if-shell", "-F", "-t", pane, &idle, &reserve])
        .await?;
    if reserved != "reserved" {
        return Err(io::Error::other(
            "The agent window changed before the paste.",
        ));
    }
    server.load(buffer, text).await?;
    let ready = format!("#{{&&:{guard},#{{==:#{{@ws_seq}},{watermark_us}}}}}");
    let paste = format!("paste-buffer -d -p -r -b {buffer} -t {pane}");
    server
        .run(&["if-shell", "-F", "-t", pane, &ready, &paste])
        .await?;
    if server.run(&["show-buffer", "-b", buffer]).await.is_ok() {
        return Err(io::Error::other(
            "The agent window did not accept the paste.",
        ));
    }
    tokio::time::sleep(SUBMIT_DELAY).await;
    let enter = format!("send-keys -t {pane} Enter; display-message -p submitted");
    let sent = server
        .run(&["if-shell", "-F", "-t", pane, &ready, &enter])
        .await?;
    if sent != "submitted" {
        return Err(io::Error::other(
            "The agent window changed before submission.",
        ));
    }
    Ok(())
}

fn ownership(paste: &Paste, id: SessionId) -> String {
    format!(
        "#{{&&:#{{==:#{{@ws_session}},{id}}},#{{&&:#{{==:#{{window_id}},{}}},#{{&&:#{{||:#{{==:#{{@ws_pane}},}},#{{==:#{{@ws_pane}},{}}}}},#{{==:#{{pane_dead}},0}}}}}}}}",
        paste.window, paste.pane
    )
}

fn valid_target(value: &str, prefix: char) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

impl Server {
    async fn run(&self, args: &[&str]) -> io::Result<String> {
        crate::hooks::tmux_server(self.socket.as_deref(), self.path.as_deref(), args).await
    }

    async fn load(&self, buffer: &str, text: &str) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut command = tokio::process::Command::from(crate::tmux::client());
        if let Some(socket) = &self.socket {
            command.args([crate::tmux::socket_flag(socket), socket]);
        } else if let Some(path) = &self.path {
            command.args(["-S", path]);
        } else {
            return Err(io::Error::other("The delivery has no tmux server."));
        }
        let mut child = command
            .args(["load-buffer", "-b", buffer, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        tokio::time::timeout(Duration::from_secs(2), async {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .await?;
            let output = child.wait_with_output().await?;
            if !output.status.success() {
                return Err(io::Error::other("Cannot load the paste buffer."));
            }
            Ok(())
        })
        .await
        .map_err(|_| io::Error::other("The paste buffer command exceeded its deadline."))?
    }
}

impl Sessions {
    fn active_paste(&self, session: SessionId) -> Result<Option<Paste>> {
        let record = self.get_record(&session.to_string())?;
        let entries = SessionLog::new(&self.state, session, record.device_id)
            .read()?
            .entries;
        let mut submitted = None;
        for entry in entries.into_iter().rev() {
            match entry.event_type.as_str() {
                "paste.requeued" | "paste.acknowledged" => return Ok(None),
                "paste.submitted" => {
                    submitted = Some(
                        serde_json::from_value::<Ulid>(entry.body["batch_id"].clone())
                            .map_err(Error::Json)?,
                    );
                }
                "paste.started" => {
                    let mut paste: Paste =
                        serde_json::from_value(entry.body).map_err(Error::Json)?;
                    paste.submitted = submitted == Some(paste.batch_id);
                    return Ok(Some(paste));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn requeue_paste(&mut self, session: SessionId, batch_id: Ulid) -> Result<()> {
        if self
            .active_paste(session)?
            .is_some_and(|p| p.batch_id == batch_id)
        {
            self.append(session, "paste.requeued", json!({"batch_id":batch_id}))?;
        }
        Ok(())
    }

    pub(crate) fn acknowledge_paste(&mut self, callback: &Callback, window: &str) -> Result<()> {
        let event = callback.payload["hook_event_name"]
            .as_str()
            .or_else(|| callback.payload["hookEventName"].as_str())
            .unwrap_or("");
        if !matches!(event, "UserPromptSubmit" | "user_prompt_submit")
            || callback.state != "working"
            || callback.payload["agent_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        {
            return Ok(());
        }
        let Some(paste) = self.active_paste(callback.session_id)? else {
            return Ok(());
        };
        let same_server = if callback.tmux_socket.is_some() {
            callback.tmux_socket == paste.server.socket
        } else {
            callback.tmux_path.is_some() && callback.tmux_path == paste.server.path
        };
        if !paste.submitted
            || callback.observed_at_us <= paste.watermark_us
            || callback.observed_at_us > paste.deadline_us
            || callback.pane.as_deref() != Some(&paste.pane)
            || window != paste.window
            || !same_server
        {
            return Ok(());
        }
        let messages = self.registry.messages()?;
        for id in paste.message_ids {
            if messages
                .iter()
                .any(|m| m.id == id && (m.delivered_at_ms.is_some() || m.read_at_ms.is_some()))
            {
                continue;
            }
            self.append(
                callback.session_id,
                "message.delivered",
                json!({
                    "message_id":id, "batch_id":paste.batch_id, "callback_id":callback.callback_id,
                }),
            )?;
        }
        self.append(
            callback.session_id,
            "paste.acknowledged",
            json!({"batch_id":paste.batch_id, "callback_id":callback.callback_id}),
        )?;
        Ok(())
    }

    pub(crate) fn stop_delivery(
        &mut self,
        callback: &Callback,
        live: bool,
    ) -> Result<StopDecision> {
        let Some(_backend) = self.delivery_guard()? else {
            return Ok(StopDecision::Accept);
        };
        let session = self.get_record(&callback.session_id.to_string())?;
        // Local sessions have permanent leases until the hub lease protocol exists.
        if session.device_id != self.device_id {
            return Ok(StopDecision::Accept);
        }
        let messages = self.registry.messages()?;
        let delivered: HashSet<_> = messages
            .iter()
            .filter(|message| message.delivered_at_ms.is_some())
            .map(|message| message.id)
            .collect();
        for message in messages.into_iter().filter(|message| {
            message.recipient == session.id
                && message.delivered_at_ms.is_some()
                && message.read_at_ms.is_none()
        }) {
            self.append(
                session.id,
                "message.read",
                json!({
                    "message_id":message.id, "implicit":true, "callback_id":callback.callback_id,
                }),
            )?;
        }
        if self.active_paste(session.id)?.is_some() {
            return Ok(StopDecision::Accept);
        }
        if !live {
            return Ok(StopDecision::Accept);
        }
        let batch: Vec<_> = self
            .inbox(session.id)?
            .into_iter()
            .filter(|message| !delivered.contains(&message.event_id))
            .collect();
        if batch.is_empty() {
            return Ok(StopDecision::Accept);
        }
        let batch = Batch {
            callback_id: callback.callback_id,
            message_ids: batch.iter().map(|message| message.event_id).collect(),
            reason: batch.iter().map(render).collect::<Vec<_>>().join("\n\n"),
        };
        self.append(
            session.id,
            "hook.delivery",
            serde_json::to_value(&batch).map_err(Error::Json)?,
        )?;
        Ok(StopDecision::Block(batch.reason))
    }

    pub(crate) fn delivery_decision(
        &self,
        session: SessionId,
        callback: Ulid,
    ) -> Result<Option<StopDecision>> {
        Ok(self
            .delivery_batch(session, callback)?
            .map(|batch| StopDecision::Block(batch.reason)))
    }

    pub(crate) fn delivery_reply(
        &mut self,
        session: SessionId,
        callback: Ulid,
        reply: Value,
    ) -> Result<Value> {
        let Some(_backend) = self.delivery_guard()? else {
            return Ok(json!({}));
        };
        if self.get_record(&session.to_string())?.device_id != self.device_id {
            return Ok(json!({}));
        }
        if reply["decision"] != "block" {
            return Ok(reply);
        }
        if self.active_paste(session)?.is_some() {
            return Ok(json!({}));
        }
        let Some(batch) = self.delivery_batch(session, callback)? else {
            return Ok(reply);
        };
        let settled: HashSet<_> = self
            .registry
            .messages()?
            .into_iter()
            .filter(|message| message.delivered_at_ms.is_some() || message.read_at_ms.is_some())
            .map(|message| message.id)
            .collect();
        let pending: Vec<_> = self
            .inbox(session)?
            .into_iter()
            .filter(|message| {
                batch.message_ids.contains(&message.event_id)
                    && !settled.contains(&message.event_id)
            })
            .collect();
        if pending.is_empty() {
            return Ok(json!({}));
        }
        Ok(
            json!({"decision":"block", "reason":pending.iter().map(render).collect::<Vec<_>>().join("\n\n")}),
        )
    }

    fn confirm_delivery(&mut self, receipt: &Receipt) -> Result<()> {
        self.record_call(None, "hook.delivered", None)?;
        let session = self.get_record(&receipt.session_id.to_string())?;
        if session.device_id != self.device_id {
            return Err(Error::InvalidRequest(
                "This device does not hold the session lease.",
            ));
        }
        let batch = self
            .delivery_batch(session.id, receipt.callback_id)?
            .ok_or(Error::InvalidRequest("The hook has no delivery batch."))?;
        let messages = self.registry.messages()?;
        for id in batch.message_ids {
            if messages.iter().any(|message| {
                message.id == id
                    && (message.delivered_at_ms.is_some() || message.read_at_ms.is_some())
            }) {
                continue;
            }
            self.append(
                session.id,
                "message.delivered",
                json!({"message_id":id, "callback_id":receipt.callback_id}),
            )?;
        }
        Ok(())
    }

    fn delivery_batch(&self, session: SessionId, callback: Ulid) -> Result<Option<Batch>> {
        self.registry
            .event_for_callback(session, "hook.delivery", &callback.to_string())?
            .map(|entry| serde_json::from_value(entry.body).map_err(Error::Json))
            .transpose()
    }
}

pub(crate) fn router() -> Router<Store> {
    Router::new().route("/v1/hooks/delivered", post(confirm))
}

/// Preserves a confirmation if the daemon stops after the hook writes stdout.
pub fn spool(state: &Path, receipt: &Receipt) -> io::Result<PathBuf> {
    let directory = state.join("delivery-spool");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let path = directory.join(format!("{}.json", receipt.callback_id));
    let temporary = directory.join(format!("{}.tmp", Ulid::new()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(receipt)?)?;
    file.sync_all()?;
    fs::rename(&temporary, &path)?;
    fs::File::open(&directory)?.sync_all()?;
    Ok(path)
}

pub(crate) fn replay(sessions: &mut Sessions) -> Result<()> {
    let directory = sessions.state.join("delivery-spool");
    let paths = match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for path in paths {
        let result = (|| -> Result<()> {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            let receipt: Receipt = serde_json::from_slice(&bytes).map_err(Error::Json)?;
            sessions.confirm_delivery(&receipt)?;
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        })();
        if let Err(error) = result {
            eprintln!("Cannot replay delivery {}: {error}", path.display());
        }
    }
    Ok(())
}

async fn confirm(
    State(store): State<Store>,
    Json(receipt): Json<Receipt>,
) -> std::result::Result<Json<Value>, ApiError> {
    api::call(store, move |sessions| sessions.confirm_delivery(&receipt)).await?;
    Ok(Json(json!({})))
}

fn render(message: &Message) -> String {
    let sender = message
        .from_alias
        .map(|alias| format!("#{alias}"))
        .unwrap_or_else(|| message.content.from_session.to_string());
    let body = message
        .content
        .body
        .split('\n')
        .map(|line| {
            if line.starts_with("[ws ") {
                format!(" {line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("[ws {sender}]\n{body}")
}

#[cfg(test)]
mod tests {
    use crate::{
        api::Store,
        hooks::{Callback, StopDecision, ingest, replay, spool},
        sessions::{CreateSession, Sessions},
    };
    use axum::http::StatusCode;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use ws_core::{
        ids::{SessionId, Ulid},
        log::SessionLog,
    };

    struct Sandbox {
        home: crate::test_sandbox::SandboxRoot,
        store: Store,
        sender: SessionId,
        recipient: SessionId,
    }

    impl Sandbox {
        fn new() -> Self {
            let home = crate::test_sandbox::SandboxRoot::new();
            let mut sessions = Sessions::open(home.path(), 1).unwrap();
            let sender = sessions.create(CreateSession::default()).unwrap().id;
            let recipient = sessions.create(CreateSession::default()).unwrap().id;
            Self {
                home,
                store: Arc::new(Mutex::new(sessions)),
                sender,
                recipient,
            }
        }

        fn send(&self, body: &str) -> crate::bus::Message {
            self.store
                .lock()
                .unwrap()
                .send_message(&self.recipient.to_string(), Some(self.sender), body)
                .unwrap()
        }

        fn callback(&self, seq: u64) -> Callback {
            Callback {
                callback_id: Ulid::new(),
                session_id: self.recipient,
                observed_at_us: seq,
                state: "stop".into(),
                pane: None,
                tmux_socket: None,
                tmux_path: None,
                payload: json!({"hook_event_name":"Stop", "turn_id":"turn-1"}),
                notify: None,
            }
        }

        async fn confirm(&self, callback: &Callback) {
            use axum::{body::Body, http::Request};
            use tower::ServiceExt;
            let response = crate::api::router_with_store(self.store.clone()).oneshot(
                Request::post("/v1/hooks/delivered").header("content-type", "application/json")
                    .body(Body::from(json!({"session_id":callback.session_id, "callback_id":callback.callback_id}).to_string())).unwrap()
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        async fn stop(&self, seq: u64) -> Value {
            let callback = self.callback(seq);
            let reply = ingest(self.store.clone(), callback.clone(), StopDecision::Accept)
                .await
                .unwrap();
            if reply["decision"] == "block" {
                self.confirm(&callback).await;
            }
            reply
        }

        fn events(&self, kind: &str) -> Vec<ws_core::log::LogEntry> {
            SessionLog::new(self.home.path(), self.recipient, Ulid::nil())
                .read()
                .unwrap()
                .entries
                .into_iter()
                .filter(|entry| entry.event_type == kind)
                .collect()
        }
    }

    struct PasteSandbox {
        sandbox: Sandbox,
        server: String,
        tmux: crate::tmux::Tmux,
        window: String,
        pane: String,
    }

    impl PasteSandbox {
        async fn new() -> Self {
            let sandbox = Sandbox::new();
            let server = sandbox.home.tmux_socket();
            let tmux = crate::tmux::Tmux::named(&server);
            let mut result = Self {
                sandbox,
                server,
                tmux,
                window: String::new(),
                pane: String::new(),
            };
            let output = std::process::Command::new("tmux")
                .args([
                    "-S",
                    &result.server,
                    "-f",
                    "/dev/null",
                    "new-session",
                    "-d",
                    "-s",
                    "paste",
                    "/bin/sleep",
                    "300",
                ])
                .env("HOME", result.sandbox.home.path())
                .env("WS_HOME", result.sandbox.home.path().join("ws"))
                .env("WS_STATE", result.sandbox.home.path())
                .env("WS_SOCK", result.sandbox.home.path().join("ws.sock"))
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let script = r#"import os,sys,tty
 tty.setraw(0)
 os.write(1,b'\x1b[?2004h')
 while True:
     data=os.read(0,4096)
     with open(sys.argv[1], 'ab') as f: f.write(data)
 "#
            .replace("\n ", "\n");
            result.window = result
                .tmux
                .create_window(
                    "paste",
                    "agent",
                    &[
                        "python3",
                        "-c",
                        &script,
                        result
                            .sandbox
                            .home
                            .path()
                            .join("paste.bin")
                            .to_str()
                            .unwrap(),
                    ],
                )
                .unwrap();
            result.pane = result
                .tmux
                .execute(&["display-message", "-p", "-t", &result.window, "#{pane_id}"])
                .unwrap();
            for (key, value) in [
                ("@ws_session", result.sandbox.recipient.to_string()),
                ("@ws_pane", result.pane.clone()),
                ("@ws_state", "idle".into()),
                ("@ws_main", "idle".into()),
            ] {
                result
                    .tmux
                    .set_window_option(&result.window, key, &value)
                    .unwrap();
            }
            result.sandbox.store.lock().unwrap().append(result.sandbox.recipient, "workspace.created", json!({
                "workspace_id":"paste", "windows":[{"window_id":result.window,"session_id":result.sandbox.recipient}]
            })).unwrap();
            result
                .wait(|| {
                    result
                        .tmux
                        .execute(&[
                            "display-message",
                            "-p",
                            "-t",
                            &result.pane,
                            "#{bracket_paste_flag}",
                        ])
                        .unwrap()
                        == "1"
                })
                .await;
            result
        }

        fn worker(&self, timeout: std::time::Duration) -> super::Worker {
            super::Worker::spawn(
                self.sandbox.store.clone(),
                super::Server {
                    socket: Some(self.server.clone()),
                    path: None,
                },
                timeout,
                std::time::Duration::from_millis(10),
            )
        }

        fn controlled_worker(&self) -> (super::Worker, Arc<std::sync::atomic::AtomicU64>) {
            use std::sync::atomic::{AtomicU64, Ordering};
            let clock = Arc::new(AtomicU64::new(crate::hooks::receipt_time_us().unwrap()));
            let now = clock.clone();
            let worker = super::Worker::with_clock(
                self.sandbox.store.clone(),
                super::Server {
                    socket: Some(self.server.clone()),
                    path: None,
                },
                std::time::Duration::from_secs(30),
                std::time::Duration::from_millis(10),
                Arc::new(move || Ok(now.load(Ordering::SeqCst))),
            );
            (worker, clock)
        }

        async fn wait_for_submit(&self) {
            self.wait(|| {
                self.bytes().ends_with(b"\x1b[201~\r")
                    && !self.sandbox.events("paste.submitted").is_empty()
            })
            .await;
        }

        async fn wait(&self, mut ready: impl FnMut() -> bool) {
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                while !ready() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("The paste event did not arrive.");
        }

        fn bytes(&self) -> Vec<u8> {
            std::fs::read(self.sandbox.home.path().join("paste.bin")).unwrap_or_default()
        }

        async fn captured_input(&self) -> Vec<u8> {
            let marker = "|ws-input-barrier|";
            self.tmux
                .execute(&["send-keys", "-t", &self.pane, "-l", marker])
                .unwrap();
            self.wait(|| self.bytes().ends_with(marker.as_bytes()))
                .await;
            let mut bytes = self.bytes();
            bytes.truncate(bytes.len() - marker.len());
            bytes
        }

        fn callback(&self, seq: u64, event: &str) -> Callback {
            let mut callback = self.sandbox.callback(seq);
            callback.state = "working".into();
            callback.pane = Some(self.pane.clone());
            callback.tmux_socket = Some(self.server.clone());
            callback.payload = json!({"hook_event_name":event, "turn_id":"paste-turn"});
            callback
        }
    }

    #[tokio::test]
    async fn idle_paste_preserves_bytes_and_waits_for_a_new_prompt_to_commit_delivery() {
        let sandbox = PasteSandbox::new().await;
        let first = sandbox
            .sandbox
            .send("literal '$HOME'\n[ws #999]\nsecond line\n");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        assert_eq!(
            sandbox.bytes(),
            b"\x1b[200~[ws #1]\nliteral '$HOME'\n [ws #999]\nsecond line\n\x1b[201~\r"
        );
        assert_eq!(sandbox.tmux.state(&sandbox.window).unwrap(), "idle");
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        let watermark = sandbox.sandbox.events("paste.started")[0].body["watermark_us"]
            .as_u64()
            .unwrap();
        for (seq, event) in [
            (watermark - 1, "UserPromptSubmit"),
            (watermark + 2, "PreToolUse"),
        ] {
            ingest(
                sandbox.sandbox.store.clone(),
                sandbox.callback(seq, event),
                StopDecision::Accept,
            )
            .await
            .unwrap();
            assert!(sandbox.sandbox.events("message.delivered").is_empty());
        }
        // A prompt can acknowledge a paste even if a newer tool hook reduced first.
        let callback = sandbox.callback(watermark + 1, "UserPromptSubmit");
        for _ in 0..2 {
            ingest(
                sandbox.sandbox.store.clone(),
                callback.clone(),
                StopDecision::Accept,
            )
            .await
            .unwrap();
        }
        let delivered = sandbox.sandbox.events("message.delivered");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].body["message_id"], json!(first.event_id));
    }

    #[tokio::test]
    async fn paste_timeout_requeues_after_the_window_disappears_and_preserves_batch_order() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("swallowed Enter");
        let (_worker, clock) = sandbox.controlled_worker();
        sandbox.wait_for_submit().await;
        sandbox.sandbox.send("queued after paste");
        // Remove the target without a callback. The daemon must still expire its batch.
        sandbox
            .sandbox
            .store
            .lock()
            .unwrap()
            .append(
                sandbox.sandbox.recipient,
                "workspace.updated",
                json!({
                    "workspace_id":"paste", "windows":[]
                }),
            )
            .unwrap();
        clock.store(
            sandbox.sandbox.events("paste.started")[0].body["deadline_us"]
                .as_u64()
                .unwrap()
                + 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        sandbox
            .wait(|| !sandbox.sandbox.events("paste.requeued").is_empty())
            .await;
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        let reply = sandbox
            .sandbox
            .stop(crate::hooks::receipt_time_us().unwrap())
            .await;
        assert_eq!(
            reply["reason"],
            "[ws #1]\nswallowed Enter\n\n[ws #1]\nqueued after paste"
        );
        assert_eq!(sandbox.sandbox.events("message.delivered").len(), 2);
    }

    #[tokio::test]
    async fn a_prompt_identifies_the_same_tmux_server_by_its_socket_path() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("confirm through the pane environment");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        let mut callback =
            sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "UserPromptSubmit");
        callback.tmux_socket = None;
        callback.tmux_path = Some(
            sandbox
                .tmux
                .execute(&[
                    "display-message",
                    "-p",
                    "-t",
                    &sandbox.pane,
                    "#{socket_path}",
                ])
                .unwrap(),
        );
        ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(sandbox.sandbox.events("message.delivered").len(), 1);
    }

    #[tokio::test]
    async fn a_cached_stop_reply_cannot_inject_a_batch_that_now_waits_for_paste_acknowledgement() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("one delivery path");
        let callback = sandbox.sandbox.callback(100);
        let prepared = ingest(
            sandbox.sandbox.store.clone(),
            callback.clone(),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(prepared["decision"], "block");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        let retried = ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(retried, json!({}));
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
    }

    // Golden scenarios: ws/tests/send.test.sh and ws/tests/inflight.test.sh.
    #[tokio::test]
    async fn an_idle_split_window_receives_input_only_in_its_recorded_agent_pane() {
        let sandbox = PasteSandbox::new().await;
        let other = sandbox
            .tmux
            .create_pane(&sandbox.window, &["/bin/cat"])
            .unwrap();
        sandbox
            .tmux
            .execute(&["select-pane", "-t", &other])
            .unwrap();
        sandbox.sandbox.send("only the agent pane");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        assert_eq!(
            sandbox.bytes(),
            b"\x1b[200~[ws #1]\nonly the agent pane\x1b[201~\r"
        );
        assert!(
            !sandbox
                .tmux
                .capture_pane(&other)
                .unwrap()
                .contains("only the agent pane")
        );
    }

    #[tokio::test]
    async fn ownership_loss_after_paste_prevents_enter_and_requeues_the_batch() {
        let sandbox = PasteSandbox::new().await;
        sandbox
            .tmux
            .execute(&[
                "set-hook",
                "-w",
                "-t",
                &sandbox.window,
                "after-paste-buffer",
                &format!("set-option -w -t {} @ws_session removed", sandbox.window),
            ])
            .unwrap();
        sandbox.sandbox.send("retain after the agent exits");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox
            .wait(|| !sandbox.sandbox.events("paste.requeued").is_empty())
            .await;
        assert_eq!(
            sandbox.captured_input().await,
            b"\x1b[200~[ws #1]\nretain after the agent exits\x1b[201~"
        );
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        assert_eq!(
            sandbox
                .sandbox
                .store
                .lock()
                .unwrap()
                .inbox(sandbox.sandbox.recipient)
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn a_timeout_retries_the_ordered_batch_and_an_old_prompt_cannot_ack_the_retry() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("first");
        let (_worker, clock) = sandbox.controlled_worker();
        sandbox.wait_for_submit().await;
        let first = sandbox.sandbox.events("paste.started")[0].body.clone();
        sandbox.sandbox.send("second");
        clock.store(
            first["deadline_us"].as_u64().unwrap() + 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        sandbox
            .wait(|| {
                sandbox.sandbox.events("paste.submitted").len() >= 2
                    && sandbox.bytes().ends_with(b"second\x1b[201~\r")
            })
            .await;
        let old = sandbox.callback(
            first["watermark_us"].as_u64().unwrap() + 1,
            "UserPromptSubmit",
        );
        ingest(sandbox.sandbox.store.clone(), old, StopDecision::Accept)
            .await
            .unwrap();
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        let callback = sandbox.callback(
            sandbox.sandbox.events("paste.started")[1].body["watermark_us"]
                .as_u64()
                .unwrap()
                + 1,
            "UserPromptSubmit",
        );
        ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(sandbox.sandbox.events("message.delivered").len(), 2);
        let batches = sandbox.sandbox.events("paste.started");
        assert_eq!(
            batches[1].body["message_ids"][0],
            batches[0].body["message_ids"][0]
        );
    }

    #[tokio::test]
    async fn a_background_agent_postpones_paste_until_its_stop_makes_the_window_idle() {
        let sandbox = PasteSandbox::new().await;
        sandbox.tmux.set_state(&sandbox.window, "working").unwrap();
        sandbox
            .tmux
            .set_window_option(&sandbox.window, "@ws_sub_background", "1")
            .unwrap();
        sandbox.sandbox.send("retry without another send");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        let mut callback =
            sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "PostToolUse");
        callback.payload["agent_id"] = json!("background");
        ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert!(sandbox.bytes().is_empty());
        let mut callback =
            sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "SubagentStop");
        callback.state = "subagent-stop".into();
        callback.payload["agent_id"] = json!("background");
        ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        sandbox.wait_for_submit().await;
        assert_eq!(
            sandbox.bytes(),
            b"\x1b[200~[ws #1]\nretry without another send\x1b[201~\r"
        );
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
    }

    #[tokio::test]
    async fn restart_restores_the_inflight_batch_and_a_spooled_prompt_confirms_it_once() {
        let mut sandbox = PasteSandbox::new().await;
        let message = sandbox.sandbox.send("retain across daemon restart");
        let worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        worker.shutdown().await;
        let captured = sandbox.bytes();
        let callback =
            sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "UserPromptSubmit");
        spool(sandbox.sandbox.home.path(), &callback).unwrap();
        let placeholder =
            Sessions::open(&sandbox.sandbox.home.path().join("placeholder"), 1).unwrap();
        let old = std::mem::replace(
            &mut sandbox.sandbox.store,
            Arc::new(Mutex::new(placeholder)),
        );
        drop(old);
        sandbox.sandbox.store = Arc::new(Mutex::new(
            Sessions::open(sandbox.sandbox.home.path(), 1).unwrap(),
        ));
        replay(sandbox.sandbox.store.clone(), sandbox.sandbox.home.path())
            .await
            .unwrap();
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        ingest(
            sandbox.sandbox.store.clone(),
            callback,
            StopDecision::Accept,
        )
        .await
        .unwrap();
        let delivered = sandbox.sandbox.events("message.delivered");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].body["message_id"], json!(message.event_id));
        assert_eq!(sandbox.bytes(), captured);
    }

    #[tokio::test]
    async fn sends_and_stops_wait_for_paste_acknowledgement_before_delivering_later_messages() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("in flight");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait_for_submit().await;
        let captured = sandbox.bytes();
        sandbox.sandbox.send("later");
        let mut stop = sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "Stop");
        stop.state = "stop".into();
        assert_eq!(
            ingest(sandbox.sandbox.store.clone(), stop, StopDecision::Accept)
                .await
                .unwrap(),
            json!({})
        );
        assert_eq!(sandbox.bytes(), captured);
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        let prompt = sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "UserPromptSubmit");
        ingest(sandbox.sandbox.store.clone(), prompt, StopDecision::Accept)
            .await
            .unwrap();
        assert_eq!(sandbox.sandbox.events("message.delivered").len(), 1);
        let mut stop = sandbox.callback(crate::hooks::receipt_time_us().unwrap(), "Stop");
        stop.state = "stop".into();
        let reply = ingest(
            sandbox.sandbox.store.clone(),
            stop.clone(),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(reply["reason"], "[ws #1]\nlater");
        assert_eq!(sandbox.sandbox.events("message.read").len(), 1);
        assert_eq!(
            sandbox.sandbox.events("message.read")[0].body["implicit"],
            true
        );
        sandbox.sandbox.confirm(&stop).await;
        assert_eq!(sandbox.sandbox.events("message.delivered").len(), 2);
    }

    #[tokio::test]
    async fn ownership_loss_before_paste_keeps_input_out_of_the_replacement_shell() {
        let sandbox = PasteSandbox::new().await;
        sandbox
            .tmux
            .execute(&[
                "set-hook",
                "-g",
                "after-load-buffer",
                &format!("set-option -w -t {} @ws_session removed", sandbox.window),
            ])
            .unwrap();
        sandbox.sandbox.send("retain without pane input");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox
            .wait(|| !sandbox.sandbox.events("paste.requeued").is_empty())
            .await;
        assert!(sandbox.captured_input().await.is_empty());
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        assert!(
            sandbox
                .tmux
                .execute(&["list-buffers", "-F", "#{buffer_name}"])
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_prompt_cannot_confirm_a_batch_after_a_crash_before_pane_submission() {
        let sandbox = PasteSandbox::new().await;
        let message = sandbox
            .sandbox
            .send("retain if the daemon stops before input");
        let watermark = crate::hooks::receipt_time_us().unwrap();
        // Restore the durable record from the crash point before any tmux input.
        sandbox
            .sandbox
            .store
            .lock()
            .unwrap()
            .append(
                sandbox.sandbox.recipient,
                "paste.started",
                json!({
                    "batch_id":Ulid::new(), "message_ids":[message.event_id],
                    "watermark_us":watermark, "deadline_us":watermark + 30_000_000,
                    "window":sandbox.window, "pane":sandbox.pane,
                    "server":{"socket":sandbox.server, "path":null}
                }),
            )
            .unwrap();
        let prompt = sandbox.callback(watermark + 1, "UserPromptSubmit");
        ingest(sandbox.sandbox.store.clone(), prompt, StopDecision::Accept)
            .await
            .unwrap();
        assert!(sandbox.bytes().is_empty());
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
    }

    #[tokio::test]
    async fn paste_timeout_requeues_even_if_the_session_finishes_before_acknowledgement() {
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.send("retain after finish");
        let (_worker, clock) = sandbox.controlled_worker();
        sandbox.wait_for_submit().await;
        {
            let mut sessions = sandbox.sandbox.store.lock().unwrap();
            let id = sandbox.sandbox.recipient.to_string();
            sessions
                .transition(&id, ws_core::state::SessionState::Running, None)
                .unwrap();
            sessions
                .transition(&id, ws_core::state::SessionState::Finished, None)
                .unwrap();
        }
        clock.store(
            sandbox.sandbox.events("paste.started")[0].body["deadline_us"]
                .as_u64()
                .unwrap()
                + 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        sandbox
            .wait(|| !sandbox.sandbox.events("paste.requeued").is_empty())
            .await;
        assert!(sandbox.sandbox.events("message.delivered").is_empty());
        assert_eq!(sandbox.sandbox.events("paste.started").len(), 1);
    }

    #[tokio::test]
    async fn retry_does_not_write_inbox_audits_while_the_agent_is_busy() {
        let sandbox = PasteSandbox::new().await;
        sandbox.tmux.set_state(&sandbox.window, "working").unwrap();
        let observed = sandbox.sandbox.home.path().join("busy-observed");
        sandbox
            .tmux
            .execute(&[
                "set-hook",
                "-g",
                "after-display-message",
                &format!("run-shell 'touch {}'", observed.display()),
            ])
            .unwrap();
        sandbox.sandbox.send("wait without audit churn");
        let _worker = sandbox.worker(std::time::Duration::from_secs(30));
        sandbox.wait(|| observed.exists()).await;
        assert!(sandbox.sandbox.events("api.called").is_empty());
        assert!(sandbox.sandbox.events("paste.started").is_empty());
    }

    #[tokio::test]
    async fn zsh_selection_suppresses_idle_paste_and_retains_its_messages() {
        use std::time::Duration;
        let sandbox = PasteSandbox::new().await;
        sandbox.sandbox.store.lock().unwrap().legacy_home =
            Some(sandbox.sandbox.home.path().into());
        std::fs::write(sandbox.sandbox.home.path().join("backend"), "zsh\n").unwrap();
        sandbox.sandbox.send("wait for Rust");
        let server = super::Server {
            socket: Some(sandbox.server.clone()),
            path: None,
        };
        let clock: super::Clock = Arc::new(crate::hooks::receipt_time_us);
        super::retry(
            sandbox.sandbox.store.clone(),
            &server,
            Duration::from_secs(30),
            &clock,
        )
        .await
        .unwrap();
        assert!(sandbox.sandbox.events("paste.started").is_empty());
        std::fs::write(sandbox.sandbox.home.path().join("backend"), "rust\n").unwrap();
        super::retry(
            sandbox.sandbox.store.clone(),
            &server,
            Duration::from_secs(30),
            &clock,
        )
        .await
        .unwrap();
        assert_eq!(sandbox.sandbox.events("paste.submitted").len(), 1);
    }

    #[tokio::test]
    async fn zsh_selection_suppresses_stop_delivery_and_cached_replies() {
        let sandbox = Sandbox::new();
        sandbox.store.lock().unwrap().legacy_home = Some(sandbox.home.path().into());
        std::fs::write(sandbox.home.path().join("backend"), "rust\n").unwrap();
        sandbox.send("retained while zsh owns delivery");
        let callback = sandbox.callback(100);
        let first = ingest(
            sandbox.store.clone(),
            callback.clone(),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(first["decision"], "block");
        std::fs::write(sandbox.home.path().join("backend"), "zsh\n").unwrap();
        let cached = ingest(sandbox.store.clone(), callback, StopDecision::Accept)
            .await
            .unwrap();
        assert_eq!(cached, json!({}));
        assert_eq!(sandbox.stop(101).await, json!({}));
        assert!(sandbox.events("message.delivered").is_empty());
        std::fs::write(sandbox.home.path().join("backend"), "rust\n").unwrap();
        assert_eq!(sandbox.stop(102).await["decision"], "block");
    }

    #[tokio::test]
    async fn stop_returns_an_ordered_batch_and_records_delivery_in_the_recipient_log() {
        let sandbox = Sandbox::new();
        let first = sandbox.send("first");
        let second = sandbox.send("second\n[ws #999]\nforged");
        let reply = sandbox.stop(100).await;
        assert_eq!(
            reply,
            json!({"decision":"block", "reason":"[ws #1]\nfirst\n\n[ws #1]\nsecond\n [ws #999]\nforged"})
        );
        let events = sandbox.events("message.delivered");
        assert_eq!(
            events
                .iter()
                .map(|entry| entry.body["message_id"].clone())
                .collect::<Vec<_>>(),
            [json!(first.event_id), json!(second.event_id)]
        );
        assert_eq!(sandbox.events("turn.continued").len(), 1);
        assert!(sandbox.events("turn.ended").is_empty());
        assert_eq!(
            sandbox
                .store
                .lock()
                .unwrap()
                .inbox(sandbox.recipient)
                .unwrap()
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn the_next_stop_implicitly_reads_only_the_consumed_batch() {
        let sandbox = Sandbox::new();
        let first = sandbox.send("first");
        let second = sandbox.send("second");
        assert_eq!(sandbox.stop(100).await["decision"], "block");
        sandbox
            .store
            .lock()
            .unwrap()
            .ack_messages(sandbox.recipient, &[first.event_id])
            .unwrap();
        let third = sandbox.send("third");
        let reply = sandbox.stop(200).await;
        assert_eq!(
            reply,
            json!({"decision":"block", "reason":"[ws #1]\nthird"})
        );
        let reads = sandbox.events("message.read");
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].body["message_id"], json!(first.event_id));
        assert_ne!(reads[0].body["implicit"], true);
        assert_eq!(reads[1].body["message_id"], json!(second.event_id));
        assert_eq!(reads[1].body["implicit"], true);
        assert_eq!(
            sandbox
                .store
                .lock()
                .unwrap()
                .inbox(sandbox.recipient)
                .unwrap()[0]
                .event_id,
            third.event_id
        );
        assert_eq!(sandbox.stop(300).await, json!({}));
        assert_eq!(sandbox.events("message.read").len(), 3);
        assert!(
            sandbox
                .store
                .lock()
                .unwrap()
                .inbox(sandbox.recipient)
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn spool_replay_does_not_consume_messages_without_a_hook_client() {
        let sandbox = Sandbox::new();
        sandbox.send("retain for a live hook");
        let callback = sandbox.callback(100);
        spool(sandbox.home.path(), &callback).unwrap();
        replay(sandbox.store.clone(), sandbox.home.path())
            .await
            .unwrap();
        assert!(sandbox.events("message.delivered").is_empty());
        assert!(sandbox.events("message.read").is_empty());
        let reply = ingest(
            sandbox.store.clone(),
            sandbox.callback(200),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(reply["reason"], "[ws #1]\nretain for a live hook");
    }
    #[tokio::test]
    async fn preparing_a_reply_does_not_deliver_until_the_client_writes_it() {
        let sandbox = Sandbox::new();
        sandbox.send("retain if the hook loses its reply");
        let reply = ingest(
            sandbox.store.clone(),
            sandbox.callback(100),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(reply["decision"], "block");
        assert!(sandbox.events("message.delivered").is_empty());
    }
    #[tokio::test]
    async fn a_spooled_turn_end_reads_the_consumed_batch_without_injecting_a_new_one() {
        let sandbox = Sandbox::new();
        sandbox.send("consumed");
        sandbox.stop(100).await;
        sandbox.send("wait for a live hook");
        spool(sandbox.home.path(), &sandbox.callback(200)).unwrap();
        replay(sandbox.store.clone(), sandbox.home.path())
            .await
            .unwrap();
        assert_eq!(sandbox.events("message.read").len(), 1);
        assert_eq!(sandbox.events("message.delivered").len(), 1);
        assert_eq!(
            sandbox.stop(300).await["reason"],
            "[ws #1]\nwait for a live hook"
        );
    }
    #[tokio::test]
    async fn duplicate_sync_and_callback_replay_inject_a_message_once_across_restart() {
        let sandbox = Sandbox::new();
        let message = sandbox.send("once");
        let sent = SessionLog::new(sandbox.home.path(), sandbox.sender, Ulid::nil())
            .read()
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.event_id == message.event_id)
            .unwrap();
        for _ in 0..2 {
            sandbox.store.lock().unwrap().registry.apply(&sent).unwrap();
        }
        let callback = sandbox.callback(100);
        assert_eq!(
            ingest(
                sandbox.store.clone(),
                callback.clone(),
                StopDecision::Accept
            )
            .await
            .unwrap()["decision"],
            "block"
        );
        let receipt = crate::bus::delivery::Receipt {
            session_id: sandbox.recipient,
            callback_id: callback.callback_id,
        };
        crate::bus::delivery::spool(sandbox.home.path(), &receipt).unwrap();
        spool(sandbox.home.path(), &callback).unwrap();
        let Sandbox {
            home,
            store,
            sender,
            recipient,
        } = sandbox;
        drop(store);
        let store = Arc::new(Mutex::new(Sessions::open(home.path(), 1).unwrap()));
        let sandbox = Sandbox {
            home,
            store,
            sender,
            recipient,
        };
        for _ in 0..2 {
            crate::bus::delivery::spool(sandbox.home.path(), &receipt).unwrap();
            spool(sandbox.home.path(), &callback).unwrap();
            replay(sandbox.store.clone(), sandbox.home.path())
                .await
                .unwrap();
            sandbox.store.lock().unwrap().registry.apply(&sent).unwrap();
            assert_eq!(
                ingest(
                    sandbox.store.clone(),
                    callback.clone(),
                    StopDecision::Accept
                )
                .await
                .unwrap(),
                json!({})
            );
        }
        assert_eq!(sandbox.events("message.delivered").len(), 1);
        assert_eq!(sandbox.events("hook.delivery").len(), 1);
        assert!(sandbox.events("message.read").is_empty());
        assert_eq!(sandbox.stop(200).await, json!({}));
        assert_eq!(sandbox.events("message.read").len(), 1);
    }

    #[tokio::test]
    async fn a_foreign_device_session_does_not_inject_messages() {
        let mut sandbox = Sandbox::new();
        let id = Ulid::new();
        let remote = SessionLog::new(sandbox.home.path(), id, Ulid::new())
            .writer()
            .append_current("session.created", json!({"alias":99}))
            .unwrap();
        sandbox
            .store
            .lock()
            .unwrap()
            .registry
            .apply(&remote)
            .unwrap();
        sandbox.recipient = id;
        sandbox.send("only the holder can deliver this");
        assert_eq!(sandbox.stop(100).await, json!({}));
        assert!(sandbox.events("hook.received").is_empty());
        assert!(sandbox.events("hook.delivery").is_empty());
        assert!(sandbox.events("message.delivered").is_empty());
        assert!(sandbox.events("message.read").is_empty());
    }

    #[tokio::test]
    async fn stale_and_shutdown_stops_retain_messages() {
        let sandbox = Sandbox::new();
        let mut working = sandbox.callback(200);
        working.state = "working".into();
        working.payload = json!({"hook_event_name":"UserPromptSubmit", "turn_id":"turn-1"});
        ingest(sandbox.store.clone(), working, StopDecision::Accept)
            .await
            .unwrap();
        sandbox.send("retain");
        assert_eq!(sandbox.stop(100).await, json!({}));
        for reason in ["channel_closed", "shutdown"] {
            let mut callback = sandbox.callback(300);
            callback.payload["reason"] = json!(reason);
            assert_eq!(
                ingest(sandbox.store.clone(), callback, StopDecision::Accept)
                    .await
                    .unwrap(),
                json!({})
            );
        }
        assert!(sandbox.events("message.delivered").is_empty());
        assert_eq!(sandbox.stop(400).await["reason"], "[ws #1]\nretain");
    }
    #[tokio::test]
    async fn a_live_hook_applies_a_spooled_stdout_confirmation_before_delivery() {
        let sandbox = Sandbox::new();
        sandbox.send("written once");
        let callback = sandbox.callback(100);
        assert_eq!(
            ingest(
                sandbox.store.clone(),
                callback.clone(),
                StopDecision::Accept
            )
            .await
            .unwrap()["decision"],
            "block"
        );
        crate::bus::delivery::spool(
            sandbox.home.path(),
            &crate::bus::delivery::Receipt {
                session_id: sandbox.recipient,
                callback_id: callback.callback_id,
            },
        )
        .unwrap();
        assert_eq!(sandbox.stop(200).await, json!({}));
        assert_eq!(sandbox.events("message.delivered").len(), 1);
        assert_eq!(sandbox.events("message.read").len(), 1);
    }
    #[tokio::test]
    async fn a_cached_batch_does_not_inject_after_its_device_loses_ownership() {
        let sandbox = Sandbox::new();
        sandbox.send("only the current holder can inject");
        let callback = sandbox.callback(100);
        assert_eq!(
            ingest(
                sandbox.store.clone(),
                callback.clone(),
                StopDecision::Accept
            )
            .await
            .unwrap()["decision"],
            "block"
        );
        let moved = SessionLog::new(sandbox.home.path(), sandbox.recipient, Ulid::new())
            .writer()
            .append_current("session.running", json!({}))
            .unwrap();
        sandbox
            .store
            .lock()
            .unwrap()
            .registry
            .apply(&moved)
            .unwrap();
        assert_eq!(
            ingest(sandbox.store.clone(), callback, StopDecision::Accept)
                .await
                .unwrap(),
            json!({})
        );
        assert!(sandbox.events("message.delivered").is_empty());
    }
    #[tokio::test]
    async fn a_matching_cancel_reads_the_consumed_batch_without_injecting_new_messages() {
        let sandbox = Sandbox::new();
        let mut start = sandbox.callback(100);
        start.state = "working".into();
        start.payload = json!({"hook_event_name":"UserPromptSubmit", "turn_id":"turn-1"});
        ingest(sandbox.store.clone(), start, StopDecision::Accept)
            .await
            .unwrap();
        sandbox.send("consumed");
        sandbox.stop(200).await;
        sandbox.send("queued after consumption");
        let mut cancel = sandbox.callback(300);
        cancel.state = "cancel".into();
        cancel.payload = json!({"hook_event_name":"Interrupt", "turn_id":"turn-1"});
        assert_eq!(
            ingest(sandbox.store.clone(), cancel, StopDecision::Accept)
                .await
                .unwrap(),
            json!({})
        );
        assert_eq!(sandbox.events("message.read").len(), 1);
        assert_eq!(sandbox.events("message.delivered").len(), 1);
        assert_eq!(
            sandbox
                .store
                .lock()
                .unwrap()
                .inbox(sandbox.recipient)
                .unwrap()[0]
                .content
                .body,
            "queued after consumption"
        );
    }
    #[tokio::test]
    async fn a_retried_reply_omits_messages_acknowledged_since_preparation() {
        let sandbox = Sandbox::new();
        let first = sandbox.send("acknowledged");
        let second = sandbox.send("still pending");
        let callback = sandbox.callback(100);
        assert_eq!(
            ingest(
                sandbox.store.clone(),
                callback.clone(),
                StopDecision::Accept
            )
            .await
            .unwrap()["decision"],
            "block"
        );
        sandbox
            .store
            .lock()
            .unwrap()
            .ack_messages(sandbox.recipient, &[first.event_id])
            .unwrap();
        let reply = ingest(
            sandbox.store.clone(),
            callback.clone(),
            StopDecision::Accept,
        )
        .await
        .unwrap();
        assert_eq!(
            reply,
            json!({"decision":"block", "reason":"[ws #1]\nstill pending"})
        );
        sandbox.confirm(&callback).await;
        let delivered = sandbox.events("message.delivered");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].body["message_id"], json!(second.event_id));
    }
}
