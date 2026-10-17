//! Stop hook batches commit after the client writes its reply.

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
};
use ws_core::ids::{EventId, SessionId, Ulid};

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

pub(crate) async fn replay(store: Store, state: &Path) -> io::Result<()> {
    let directory = state.join("delivery-spool");
    let paths = match fs::read_dir(&directory) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for path in paths {
        let result = async {
            let receipt: Receipt = serde_json::from_slice(&fs::read(&path)?)?;
            api::call(store.clone(), move |sessions| {
                sessions.confirm_delivery(&receipt)
            })
            .await
            .map_err(|error| io::Error::other(error.0))?;
            fs::remove_file(&path)
        }
        .await;
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

impl Sessions {
    pub(crate) fn stop_delivery(
        &mut self,
        callback: &Callback,
        live: bool,
    ) -> Result<StopDecision> {
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
        &self,
        session: SessionId,
        callback: Ulid,
        reply: Value,
    ) -> Result<Value> {
        let Some(batch) = self.delivery_batch(session, callback)? else {
            return Ok(reply);
        };
        let messages = self.registry.messages()?;
        if batch.message_ids.iter().all(|id| {
            messages.iter().any(|message| {
                message.id == *id
                    && (message.delivered_at_ms.is_some() || message.read_at_ms.is_some())
            })
        }) {
            return Ok(json!({}));
        }
        Ok(reply)
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
            if messages
                .iter()
                .any(|message| message.id == id && message.delivered_at_ms.is_some())
            {
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
