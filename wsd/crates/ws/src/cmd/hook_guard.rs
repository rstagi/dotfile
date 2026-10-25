use crate::client;
use serde_json::{Value, json};
use std::{env, path::Path, time::Duration};

pub(super) async fn run(socket: &Path, payload: Value) {
    let Some(session) = env::var("WS_SESSION_ID").ok().filter(|id| !id.is_empty()) else {
        return;
    };
    let agent = env::var("WS_LOOP_GUARD_AGENT").unwrap_or_default();
    let request = json!({"session":session, "agent":agent, "payload":payload});
    let Ok(body) = client::hook_request(
        socket,
        "/v1/loops/guard",
        request.to_string().into_bytes(),
        Duration::from_secs(4),
    )
    .await
    else {
        return;
    };
    if let Ok(reply) = serde_json::from_slice::<Value>(&body) {
        println!("{reply}");
    }
}
