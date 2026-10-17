use crate::client;
use std::{path::Path, time::Duration};
use ws_daemon::notify::DismissRequest;

/// Dismissal is best effort when the daemon or target is absent.
pub async fn run(socket: &Path, target: Option<&str>) {
    let Some(target) = target.filter(|value| !value.is_empty()) else {
        return;
    };
    if std::env::var("WS_NOTIFY").as_deref() == Ok("0") {
        return;
    }
    let request = DismissRequest::from_env(target);
    let Ok(body) = serde_json::to_vec(&request) else {
        return;
    };
    let _ = tokio::time::timeout(
        Duration::from_millis(250),
        client::request(
            socket,
            hyper::Method::POST,
            "/v1/notifications/dismiss",
            body,
        ),
    )
    .await;
}
