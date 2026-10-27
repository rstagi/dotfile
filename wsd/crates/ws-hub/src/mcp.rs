//! The remote MCP endpoint `POST /mcp`: a stateless streamable HTTP server with read-only tools.
//!
//! Each request needs `Authorization: Bearer <token>`. The tools read only the sessions table
//! and the stored export projection, so a response never holds more than a device exported.
//! Each tool call adds one audit row.

use http_body_util::{BodyExt, Full, Limited};
use hyper::{Request, Response, StatusCode, body::Bytes, body::Incoming};
use serde_json::{Value, json};

use crate::{
    identity::Result,
    registry::Registry,
    server::reply,
    sessions::HubSession,
    sync::{StoredEntry, SyncStore},
    tokens::TokenStore,
};

const PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_BODY_BYTES: usize = 1 << 20;
/// The most entries `get_session_timeline` returns when the caller gives no limit.
const DEFAULT_TIMELINE_LIMIT: usize = 200;

/// The stores that the tools read.
pub struct McpContext<'a> {
    pub tokens: &'a TokenStore,
    pub registry: &'a Registry,
    pub sync: &'a SyncStore,
}

/// Answers one `POST /mcp` request.
pub async fn handle(context: &McpContext<'_>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let name = match bearer.map(|token| context.tokens.authenticate(&token)) {
        Some(Ok(Some(name))) => name,
        Some(Err(error)) => return reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        _ => return unauthorized(),
    };
    if request.method() != hyper::Method::POST {
        return reply(StatusCode::METHOD_NOT_ALLOWED, "Use POST.");
    }
    let Ok(body) = Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
    else {
        return reply(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Cannot read the request body.",
        );
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body.to_bytes()) else {
        return rpc_error(Value::Null, -32700, "The body is not valid JSON.");
    };
    dispatch(context, &name, &message)
}

fn dispatch(context: &McpContext<'_>, token: &str, message: &Value) -> Response<Full<Bytes>> {
    let id = message.get("id").cloned();
    let Some(method) = message["method"].as_str() else {
        return rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "The message has no method.",
        );
    };
    // A message with no id is a notification: the server accepts it and answers nothing.
    let Some(id) = id else {
        return reply(StatusCode::ACCEPTED, "");
    };
    let params = &message["params"];
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "ws-hub", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_list() }),
        "tools/call" => call_tool(context, token, params),
        _ => return rpc_error(id, -32601, "Unknown method."),
    };
    rpc_result(id, result)
}

/// Runs one tool and adds its audit row. A failed tool is a result with `isError`.
fn call_tool(context: &McpContext<'_>, token: &str, params: &Value) -> Value {
    let tool = params["name"].as_str().unwrap_or_default();
    let arguments = &params["arguments"];
    let outcome = run_tool(context, tool, arguments);
    let status = if outcome.is_ok() { "ok" } else { "error" };
    let audit = context.tokens.record_call(token, tool, status);
    let (text, is_error) = match (outcome, audit) {
        (_, Err(error)) => (format!("The hub cannot audit the call: {error}"), true),
        (Ok(value), Ok(())) => (value.to_string(), false),
        (Err(error), Ok(())) => (error.to_string(), true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn run_tool(context: &McpContext<'_>, tool: &str, arguments: &Value) -> Result<Value> {
    match tool {
        "list_sessions" => {
            let state = arguments["state"].as_str();
            let rows: Vec<_> = session_rows(context.registry, context.sync)?
                .into_iter()
                .filter(|row| state.is_none_or(|state| row.state == state))
                .collect();
            Ok(serde_json::to_value(rows)?)
        }
        "get_session" => Ok(serde_json::to_value(find_session(context, arguments)?)?),
        "get_session_timeline" => {
            let session = find_session(context, arguments)?;
            let limit = arguments["limit"]
                .as_u64()
                .map_or(DEFAULT_TIMELINE_LIMIT, |limit| limit as usize);
            let mut entries = context.sync.timeline(&session.device_id, &session.id)?;
            let skip = entries.len().saturating_sub(limit);
            entries.drain(..skip);
            Ok(json!({ "session": session.id, "entries": entries }))
        }
        _ => Err(format!("Unknown tool {tool}.").into()),
    }
}

/// The sessions table with the registry name of each device.
pub(crate) fn session_rows(registry: &Registry, sync: &SyncStore) -> Result<Vec<HubSession>> {
    let devices = registry.list()?;
    let mut sessions = sync.sessions()?;
    for session in &mut sessions {
        if let Some(device) = devices.iter().find(|device| device.id == session.device_id) {
            session.device_name = device.name.clone();
        }
    }
    Ok(sessions)
}

/// The session with `id`. When two devices hold the id, `device` (a name or id) picks one.
fn find_session(context: &McpContext<'_>, arguments: &Value) -> Result<HubSession> {
    let id = arguments["id"]
        .as_str()
        .ok_or("The argument id is missing.")?;
    let device = arguments["device"].as_str();
    session_rows(context.registry, context.sync)?
        .into_iter()
        .find(|row| {
            row.id == id
                && device.is_none_or(|device| row.device_id == device || row.device_name == device)
        })
        .ok_or_else(|| format!("No session {id}.").into())
}

fn tool_list() -> Value {
    let id = json!({ "type": "string", "description": "The session id." });
    let device = json!({ "type": "string", "description": "The device name or id. Optional." });
    json!([
        {
            "name": "list_sessions",
            "description": "List the sessions of all devices, newest event first.",
            "inputSchema": {
                "type": "object",
                "properties": { "state": { "type": "string", "description": "Show only this state." } },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "get_session",
            "description": "Show one session: name, device, state, working directory and repositories.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id, "device": device },
                "required": ["id"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "get_session_timeline",
            "description": "Show the synced events of one session in order. The events hold no secrets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id,
                    "device": device,
                    "limit": { "type": "integer", "description": "The newest N events. The default is 200." },
                },
                "required": ["id"],
            },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

impl SyncStore {
    /// The stored entries of one session in sequence order.
    pub(crate) fn timeline(&self, device_id: &str, session_id: &str) -> Result<Vec<StoredEntry>> {
        Ok(self
            .entries(device_id)?
            .into_iter()
            .filter(|stored| stored.entry.session_id == session_id)
            .collect())
    }
}

fn rpc_result(id: Value, result: Value) -> Response<Full<Bytes>> {
    crate::server::json(
        StatusCode::OK,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn rpc_error(id: Value, code: i32, message: &str) -> Response<Full<Bytes>> {
    crate::server::json(
        StatusCode::OK,
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    )
}

fn unauthorized() -> Response<Full<Bytes>> {
    let mut response = reply(
        StatusCode::UNAUTHORIZED,
        "A valid bearer token is required.",
    );
    response
        .headers_mut()
        .insert("www-authenticate", "Bearer".parse().unwrap());
    response
}
