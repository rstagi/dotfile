//! Dependency graph mutations are serialized with log publication.

use std::collections::{HashMap, VecDeque};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use ws_core::{ids::SessionId, registry::Edge};

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddEdge {
    target: String,
    parent_session_id: Option<SessionId>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/edges", get(list))
        .route("/v1/sessions/{id}/depends_on", post(depends_on))
        .route("/v1/sessions/{id}/block_on", post(block_on))
}

async fn list(State(store): State<Store>) -> std::result::Result<Json<Vec<Edge>>, ApiError> {
    Ok(Json(
        api::call(store, |sessions| Ok(sessions.registry.edges()?)).await?,
    ))
}

async fn depends_on(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<AddEdge>,
) -> std::result::Result<(StatusCode, Json<Edge>), ApiError> {
    add(store, id, request, "depends_on").await
}

async fn block_on(
    State(store): State<Store>,
    Path(id): Path<String>,
    Json(request): Json<AddEdge>,
) -> std::result::Result<(StatusCode, Json<Edge>), ApiError> {
    add(store, id, request, "block_on").await
}

async fn add(
    store: Store,
    id: String,
    request: AddEdge,
    kind: &'static str,
) -> std::result::Result<(StatusCode, Json<Edge>), ApiError> {
    let edge = api::call(store, move |sessions| {
        sessions.add_dependency(&id, &request.target, kind, request.parent_session_id)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(edge)))
}

impl Sessions {
    pub fn add_dependency(
        &mut self,
        source: &str,
        target: &str,
        kind: &str,
        caller: Option<SessionId>,
    ) -> Result<Edge> {
        self.record_call(caller, "session.edge", Some(source))?;
        if !["depends_on", "block_on"].contains(&kind) {
            return Err(Error::InvalidRequest(
                "Use depends_on or block_on for a dependency.",
            ));
        }
        let source = self.get_record(source)?.id;
        let target = self.get_record(target)?.id;
        if let Some(mut cycle) = path(&self.registry.edges()?, target, source) {
            cycle.insert(0, source);
            return Err(Error::DependencyCycle(cycle));
        }
        let body = json!({"target": target, "parent_session_id": caller});
        let entry = self.append_entry(source, &format!("session.{kind}"), body.clone())?;
        Ok(Edge {
            id: entry.event_id,
            source,
            target,
            kind: kind.into(),
            body,
        })
    }
}

fn path(edges: &[Edge], start: SessionId, end: SessionId) -> Option<Vec<SessionId>> {
    let mut adjacent: HashMap<SessionId, Vec<SessionId>> = HashMap::new();
    for edge in edges {
        adjacent.entry(edge.source).or_default().push(edge.target);
    }
    let mut queue = VecDeque::from([start]);
    let mut previous = HashMap::from([(start, start)]);
    while let Some(current) = queue.pop_front() {
        if current == end {
            let mut path = vec![end];
            while *path.last()? != start {
                path.push(previous[path.last()?]);
            }
            path.reverse();
            return Some(path);
        }
        for next in adjacent.get(&current).into_iter().flatten() {
            if let std::collections::hash_map::Entry::Vacant(entry) = previous.entry(*next) {
                entry.insert(current);
                queue.push_back(*next);
            }
        }
    }
    None
}
