//! Messages are sender log events. The inbox is a rebuildable projection.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ws_core::ids::{EventId, SessionId};

#[path = "delivery.rs"]
pub mod delivery;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Message {
    pub event_id: EventId,
    #[serde(flatten)]
    pub content: Content,
    pub from_alias: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Content {
    pub from_session: SessionId,
    pub to_session: SessionId,
    pub pair_seq: u64,
    pub body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendRequest {
    target: String,
    body: String,
    parent_session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxRequest {
    session_id: Option<SessionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AckRequest {
    session_id: Option<SessionId>,
    #[serde(default)]
    event_ids: Vec<EventId>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .merge(delivery::router())
        .route("/v1/messages", post(send))
        .route("/v1/inbox", get(inbox))
        .route("/v1/ack", post(ack))
}

async fn send(
    State(store): State<Store>,
    Json(request): Json<SendRequest>,
) -> std::result::Result<Json<Message>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions.send_message(&request.target, request.parent_session_id, &request.body)
        })
        .await?,
    ))
}

async fn inbox(
    State(store): State<Store>,
    Query(request): Query<InboxRequest>,
) -> std::result::Result<Json<Vec<Message>>, ApiError> {
    Ok(Json(
        api::call(store, move |sessions| {
            sessions
                .message_session(request.session_id)
                .and_then(|session| sessions.inbox(session.id))
        })
        .await?,
    ))
}

async fn ack(
    State(store): State<Store>,
    Json(request): Json<AckRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let count = api::call(store, move |sessions| {
        sessions
            .message_session(request.session_id)
            .and_then(|session| sessions.ack_messages(session.id, &request.event_ids))
    })
    .await?;
    Ok(Json(json!({"acked": count})))
}

impl Sessions {
    pub fn send_message(
        &mut self,
        target: &str,
        from: Option<SessionId>,
        body: &str,
    ) -> Result<Message> {
        self.record_call(from, "message.send", Some(target))?;
        let to = self.get_record(target)?;
        if body.is_empty() {
            return Err(Error::InvalidRequest("send: empty message"));
        }
        let from = self.message_session(from)?;
        let pair_seq = self
            .registry
            .messages()?
            .iter()
            .filter(|message| message.sender == Some(from.id) && message.recipient == to.id)
            .filter_map(|message| message.body.as_ref()?.get("pair_seq")?.as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let content = Content {
            from_session: from.id,
            to_session: to.id,
            pair_seq,
            body: body.into(),
        };
        let entry = self.append_entry(
            from.id,
            "message.sent",
            serde_json::to_value(&content).map_err(Error::Json)?,
        )?;
        if to.device_id == self.device_id
            && matches!(to.state.as_str(), "finished" | "failed")
            && let Err(error) = self.reopen_for_message(&to, Some(from.id))
        {
            // The message is durable even when its recipient cannot resume yet.
            eprintln!(
                "Cannot reopen session {} for message delivery: {error}",
                to.id
            );
        }
        Ok(Message {
            event_id: entry.event_id,
            content,
            from_alias: Some(from.alias),
        })
    }

    pub fn inbox(&mut self, session: SessionId) -> Result<Vec<Message>> {
        self.record_call(Some(session), "message.inbox", None)?;
        let aliases: std::collections::HashMap<_, _> = self
            .registry
            .sessions()?
            .into_iter()
            .map(|session| (session.id, session.alias))
            .collect();
        let mut messages = self
            .registry
            .messages()?
            .into_iter()
            .filter(|message| message.recipient == session && message.sender.is_some())
            .map(|message| {
                let content: Content = serde_json::from_value(message.body.unwrap_or_default())
                    .map_err(Error::Json)?;
                Ok((
                    Message {
                        event_id: message.id,
                        from_alias: message
                            .sender
                            .and_then(|sender| aliases.get(&sender).copied()),
                        content,
                    },
                    message.read_at_ms.is_some(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        messages.sort_by_key(|(message, _)| (message.content.pair_seq, message.event_id));
        let mut next = std::collections::HashMap::new();
        Ok(messages
            .into_iter()
            .filter_map(|(message, read)| {
                let expected = next.entry(message.content.from_session).or_insert(1);
                if message.content.pair_seq != *expected {
                    return None;
                }
                *expected += 1;
                (!read).then_some(message)
            })
            .collect())
    }

    pub fn ack_messages(&mut self, session: SessionId, ids: &[EventId]) -> Result<usize> {
        self.record_call(Some(session), "message.ack", None)?;
        let messages = self.registry.messages()?;
        if ids.iter().any(|id| {
            !messages.iter().any(|message| {
                message.id == *id && message.recipient == session && message.sender.is_some()
            })
        }) {
            return Err(Error::InvalidRequest(
                "The message does not belong to this inbox.",
            ));
        }
        let visible = self.inbox(session)?;
        if ids.iter().any(|id| {
            !visible.iter().any(|message| message.event_id == *id)
                && !messages.iter().any(|message| {
                    message.id == *id
                        && message.recipient == session
                        && message.read_at_ms.is_some()
                })
        }) {
            return Err(Error::InvalidRequest(
                "The message waits for an earlier message.",
            ));
        }
        let pending: Vec<_> = visible
            .into_iter()
            .filter(|message| ids.is_empty() || ids.contains(&message.event_id))
            .collect();
        for message in &pending {
            self.append(
                session,
                "message.read",
                json!({"message_id": message.event_id}),
            )?;
        }
        Ok(pending.len())
    }

    fn message_session(
        &mut self,
        session: Option<SessionId>,
    ) -> Result<ws_core::registry::Session> {
        self.record_call(None, "message.session", None)?;
        if let Some(session) = session {
            return self.get_record(&session.to_string());
        }
        if let Some(human) = self
            .registry
            .sessions()?
            .into_iter()
            .find(|session| session.body["human_device_id"] == self.device_id.to_string())
        {
            return Ok(human);
        }
        let id = ws_core::ids::Ulid::new();
        let alias = self.registry.next_alias()?;
        self.append(
            id,
            "session.created",
            json!({"alias":alias, "name":"human", "human_device_id":self.device_id}),
        )?;
        self.get_record(&id.to_string())
    }
}
