//! One-shot actions run when every source records the required event.

use crate::{
    api::{self, ApiError, Store},
    sessions::{Error, Result, Sessions},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use ws_core::{
    ids::{SessionId, Ulid},
    log::{LogEntry, SessionLog},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Trigger {
    pub id: Ulid,
    pub owner: SessionId,
    pub sources: Vec<SessionId>,
    pub event: String,
    pub action: String,
    pub target: Option<SessionId>,
    pub template: String,
    pub agent: Option<String>,
    pub workspace: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub spawned_session: Option<SessionId>,
    pub event_offsets: BTreeMap<SessionId, u64>,
    pub edges: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    sources: Vec<String>,
    event: String,
    target: Option<String>,
    spawn: Option<String>,
    template: Option<String>,
    workspace: Option<String>,
    cwd: Option<String>,
    parent_session_id: Option<SessionId>,
}

pub(crate) fn router() -> Router<Store> {
    Router::new()
        .route("/v1/triggers", get(list).post(register))
        .route("/v1/triggers/{id}/remove", axum::routing::post(remove))
}

async fn register(
    State(store): State<Store>,
    Json(request): Json<Register>,
) -> std::result::Result<(StatusCode, Json<Trigger>), ApiError> {
    let trigger = api::call(store, move |sessions| sessions.register_trigger(request)).await?;
    Ok((StatusCode::CREATED, Json(trigger)))
}

async fn list(State(store): State<Store>) -> std::result::Result<Json<Vec<Trigger>>, ApiError> {
    Ok(Json(
        api::call(store, |sessions| {
            Ok(sessions.triggers.values().cloned().collect())
        })
        .await?,
    ))
}

async fn remove(
    State(store): State<Store>,
    Path(id): Path<Ulid>,
    Json(context): Json<api::CallContext>,
) -> std::result::Result<Json<Value>, ApiError> {
    api::call(store, move |sessions| {
        sessions.record_call(
            context.parent_session_id,
            "trigger.remove",
            Some(&id.to_string()),
        )?;
        let trigger = sessions
            .triggers
            .get(&id)
            .cloned()
            .ok_or(Error::InvalidRequest(
                "There is no pending trigger with this id.",
            ))?;
        sessions.append(
            trigger.owner,
            "edge.action_removed",
            json!({"trigger_id":id}),
        )
    })
    .await?;
    Ok(Json(json!({})))
}

impl Sessions {
    fn register_trigger(&mut self, request: Register) -> Result<Trigger> {
        self.record_call(request.parent_session_id, "trigger.register", None)?;
        if !["done", "exit"].contains(&request.event.as_str()) {
            return Err(Error::InvalidRequest(
                "The trigger event must be done or exit.",
            ));
        }
        if request.target.is_some() == request.spawn.is_some() {
            return Err(Error::InvalidRequest(
                "Specify one send target or one spawn agent.",
            ));
        }
        let mut sources = Vec::new();
        for source in request.sources {
            let id = self.get_record(&source)?.id;
            if !sources.contains(&id) {
                sources.push(id);
            }
        }
        if sources.is_empty() {
            return Err(Error::InvalidRequest(
                "Specify at least one source session.",
            ));
        }
        let target = request
            .target
            .map(|target| self.get_record(&target).map(|record| record.id))
            .transpose()?;
        let template = request
            .template
            .unwrap_or_else(|| format!("Session {{id}} recorded {}.\n{{output}}", request.event));
        if template.replace("{output}", "").trim().is_empty() {
            return Err(Error::InvalidRequest(
                "Include text or {id} with {output} in the template.",
            ));
        }
        if request
            .spawn
            .as_deref()
            .is_some_and(|agent| !["claude", "codex", "grok"].contains(&agent))
        {
            return Err(Error::InvalidRequest(
                "The spawn agent must be claude, codex or grok.",
            ));
        }
        // The human device session owns actions registered outside an agent session.
        let owner = self.message_session(request.parent_session_id)?;
        let workspace = request.workspace.or(owner.workspace_id.clone());
        let cwd = if request.spawn.is_some() && workspace.is_none() {
            let directory = request
                .cwd
                .or(owner.cwd.clone())
                .ok_or(Error::InvalidRequest(
                    "A spawn trigger requires a workspace or a directory.",
                ))?;
            let directory = std::path::Path::new(&directory)
                .canonicalize()
                .map_err(|_| Error::InvalidRequest("The spawn directory does not exist."))?;
            if !directory.is_dir() {
                return Err(Error::InvalidRequest("The spawn directory does not exist."));
            }
            Some(
                directory
                    .to_str()
                    .ok_or(Error::InvalidRequest("Use a directory with a UTF-8 path."))?
                    .to_owned(),
            )
        } else {
            None
        };
        if request.spawn.is_some()
            && let Some(workspace) = &workspace
            && !self.registry.workspaces()?.iter().any(|record| {
                &record.id == workspace
                    && record.body["path"]
                        .as_str()
                        .is_some_and(|path| std::path::Path::new(path).is_dir())
            })
        {
            return Err(Error::InvalidRequest("The spawn workspace does not exist."));
        }
        let id = Ulid::new();
        let spawned_session = request.spawn.as_ref().map(|_| Ulid::new());
        let dependent = target.or(spawned_session).unwrap_or(owner.id);
        let edges: Vec<_> = sources.iter().filter(|source| request.event != "exit" || **source != dependent)
            .map(|source| json!({"id":Ulid::new(), "source":dependent, "target":source, "kind":"block_on", "trigger_id":id, "owner":owner.id})).collect();
        self.check_trigger_cycles(&edges)?;
        let mut offsets = BTreeMap::new();
        for source in &sources {
            let seq = self
                .registry
                .latest_event(*source, "exit")?
                .map_or(0, |entry| entry.seq);
            offsets.insert(*source, seq);
        }
        let trigger = Trigger {
            id,
            owner: owner.id,
            sources,
            event: request.event,
            action: if target.is_some() {
                "send".into()
            } else {
                "spawn".into()
            },
            target,
            template,
            agent: request.spawn,
            workspace,
            cwd,
            spawned_session,
            event_offsets: offsets,
            edges,
        };
        self.append(
            owner.id,
            "edge.action_registered",
            serde_json::to_value(&trigger).map_err(Error::Json)?,
        )?;
        self.evaluate_trigger(&trigger)?;
        Ok(trigger)
    }

    fn check_trigger_cycles(&self, additions: &[Value]) -> Result<()> {
        let mut adjacent: BTreeMap<SessionId, Vec<SessionId>> = BTreeMap::new();
        for edge in self.registry.edges()? {
            adjacent.entry(edge.source).or_default().push(edge.target);
        }
        for edge in additions {
            let source: SessionId =
                serde_json::from_value(edge["source"].clone()).map_err(Error::Json)?;
            let target: SessionId =
                serde_json::from_value(edge["target"].clone()).map_err(Error::Json)?;
            let mut stack = vec![(target, vec![source, target])];
            let mut seen = HashSet::new();
            while let Some((node, path)) = stack.pop() {
                if node == source {
                    return Err(Error::DependencyCycle(path));
                }
                if !seen.insert(node) {
                    continue;
                }
                for next in adjacent.get(&node).into_iter().flatten() {
                    let mut path = path.clone();
                    path.push(*next);
                    stack.push((*next, path));
                }
            }
            adjacent.entry(source).or_default().push(target);
        }
        Ok(())
    }

    pub(crate) fn restore_triggers(&mut self) -> Result<()> {
        self.triggers.clear();
        for session in self.registry.sessions()? {
            if session.device_id != self.device_id {
                continue;
            }
            for entry in SessionLog::new(&self.state, session.id, session.device_id)
                .read()?
                .entries
            {
                self.apply_trigger_entry(&entry);
            }
        }
        Ok(())
    }

    pub(crate) fn apply_trigger_entry(&mut self, entry: &LogEntry) {
        apply_trigger_entry(&mut self.triggers, entry);
    }

    pub(crate) fn evaluate_triggers(&mut self) -> Result<()> {
        let Some(_backend) = self.delivery_guard()? else {
            self.trigger_events_pending = true;
            return Ok(());
        };
        let pending: Vec<_> = self.triggers.values().cloned().collect();
        for trigger in pending {
            if let Err(error) = self.evaluate_trigger(&trigger) {
                eprintln!("Trigger {} failed: {error}", trigger.id);
            }
        }
        Ok(())
    }

    fn evaluate_trigger(&mut self, trigger: &Trigger) -> Result<()> {
        for source in &trigger.sources {
            // A self-send must wait for the launch wrapper, not its pre-exit hook.
            let process_exit = trigger.event == "exit"
                && trigger.target == Some(*source)
                && self.get_record(&source.to_string())?.body["agent"].is_string();
            let event = if process_exit {
                self.registry.latest_process_exit(*source)?
            } else {
                self.registry.latest_event(*source, &trigger.event)?
            };
            if event.is_none_or(|entry| {
                trigger.event == "exit"
                    && entry.seq <= trigger.event_offsets.get(source).copied().unwrap_or(0)
            }) {
                return Ok(());
            }
        }
        if let Err(error) = self.fire_trigger(trigger) {
            self.append(
                trigger.owner,
                "edge.action_failed",
                json!({"trigger_id":trigger.id, "error":error.to_string()}),
            )?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn finish_spawned_session(
        &mut self,
        session: &ws_core::registry::Session,
    ) -> Result<()> {
        match self.agent_directory(session) {
            Ok(directory) => {
                crate::agents::slots::remove(&self.state, &directory, &session.id.to_string())?
            }
            Err(Error::InvalidRequest(_)) => {}
            Err(error) => return Err(error),
        }
        let tmux = crate::tmux::Tmux::from_env();
        let id = session.id.to_string();
        let alias = session.alias.to_string();
        for window in self.session_windows(session)? {
            let owner = tmux.window_option(&window, "@ws_session").ok().flatten();
            if owner.as_deref() == Some(&id) || owner.as_deref() == Some(&alias) {
                tmux.execute(&["kill-window", "-t", &window])?;
            }
        }
        Ok(())
    }

    fn fire_trigger(&mut self, trigger: &Trigger) -> Result<()> {
        let mut outputs = Vec::new();
        for source in &trigger.sources {
            let path = self
                .state
                .join("sessions")
                .join(source.to_string())
                .join("output.md");
            let output = match std::fs::read_to_string(path) {
                Ok(output) => output,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error.into()),
            };
            outputs.push(output);
        }
        let ids = trigger
            .sources
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let message = expand(&trigger.template, &ids, &outputs.join("\n"));
        if message.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "The expanded message or prompt is empty.",
            ));
        }
        if let Some(target) = trigger.target {
            let recipient = self.get_record(&target.to_string())?;
            if trigger.event == "exit"
                && trigger.sources.contains(&target)
                && recipient.body["agent"].is_string()
                && !["finished", "failed"].contains(&recipient.state.as_str())
                && let Some(exit) = self.registry.latest_process_exit(target)?
            {
                // Failed interactive launches retain slots until an explicit continuation.
                let state = if exit.body["code"] == 0 {
                    ws_core::state::SessionState::Finished
                } else {
                    ws_core::state::SessionState::Failed
                };
                self.transition(&target.to_string(), state, Some(trigger.owner))?;
            }
            self.send_trigger_message(&target.to_string(), trigger.owner, &message, trigger.id)?;
        } else {
            let place = if let Some(workspace) = &trigger.workspace {
                let record = self
                    .registry
                    .workspaces()?
                    .into_iter()
                    .find(|record| &record.id == workspace)
                    .ok_or(Error::InvalidRequest(
                        "The spawn workspace no longer exists.",
                    ))?;
                let path = record.body["path"]
                    .as_str()
                    .ok_or(Error::InvalidRequest("The spawn workspace has no path."))?;
                crate::places::Place::Workspace {
                    name: workspace.clone(),
                    directory: path.into(),
                }
            } else {
                let directory = trigger
                    .cwd
                    .as_deref()
                    .ok_or(Error::InvalidRequest("The spawn trigger has no directory."))?;
                if !std::path::Path::new(directory).is_dir() {
                    return Err(Error::InvalidRequest(
                        "The spawn directory no longer exists.",
                    ));
                }
                crate::places::Place::Directory {
                    directory: directory.into(),
                }
            };
            let child = trigger
                .spawned_session
                .ok_or(Error::InvalidRequest("The spawn trigger has no child id."))?;
            // Commit the claim before tmux can start the child. Replay never launches it twice.
            self.append(
                trigger.owner,
                "edge.action_fired",
                json!({"trigger_id":trigger.id, "spawned_session":child}),
            )?;
            self.start_place_agent(
                place,
                trigger.agent.as_deref().unwrap_or("claude"),
                Some(trigger.owner),
                crate::workspaces::LaunchContext {
                    resume: None,
                    run: None,
                    start: None,
                    spawn: Some(crate::workspaces::SpawnOrigin {
                        id: child,
                        trigger_id: trigger.id,
                        prompt: &message,
                    }),
                },
            )?;
        }
        Ok(())
    }
}

fn expand(template: &str, ids: &str, output: &str) -> String {
    let mut expanded = String::new();
    let mut rest = template;
    while let Some(index) = rest.find('{') {
        expanded.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(next) = rest.strip_prefix("{id}") {
            expanded.push_str(ids);
            rest = next;
        } else if let Some(next) = rest.strip_prefix("{output}") {
            expanded.push_str(output);
            rest = next;
        } else {
            expanded.push('{');
            rest = &rest[1..];
        }
    }
    expanded.push_str(rest);
    expanded
}

/// Applies one entry to the triggers of its session. Each log changes only the triggers it owns.
pub(crate) fn apply_trigger_entry(triggers: &mut BTreeMap<Ulid, Trigger>, entry: &LogEntry) {
    match entry.event_type.as_str() {
        "edge.action_registered" => match serde_json::from_value::<Trigger>(entry.body.clone()) {
            Ok(trigger) if trigger.owner == entry.session_id => {
                triggers.insert(trigger.id, trigger);
            }
            _ => eprintln!(
                "Cannot read the trigger definition in event {}.",
                entry.event_id
            ),
        },
        "edge.action_removed" | "edge.action_fired" | "edge.action_failed" | "message.sent" => {
            if let Some(id) = entry.body["trigger_id"]
                .as_str()
                .and_then(|s| s.parse().ok())
                && triggers
                    .get(&id)
                    .is_some_and(|trigger| trigger.owner == entry.session_id)
            {
                triggers.remove(&id);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::sessions::Sessions;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn replay_restores_a_trigger_record_without_cwd() {
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let owner = sessions.create(Default::default()).unwrap();
        let source = sessions.create(Default::default()).unwrap();
        let id = ws_core::ids::Ulid::new();
        sessions
            .append(
                owner.id,
                "edge.action_registered",
                json!({
                    "id":id, "owner":owner.id, "sources":[source.id], "event":"done",
                    "action":"send", "target":owner.id, "template":"Continue {id}",
                    "agent":null, "workspace":null, "spawned_session":null,
                    "event_offsets":{}, "edges":[]
                }),
            )
            .unwrap();
        drop(sessions);
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (status, triggers) = request(&app, "GET", "/v1/triggers", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(triggers.as_array().unwrap().len(), 1);
        assert_eq!(triggers[0]["id"], id.to_string());
        assert_eq!(triggers[0]["cwd"], Value::Null);
    }

    #[tokio::test]
    async fn a_restart_restores_unindexed_entries_and_rebuilds_a_diverged_index() {
        use ws_core::{log::SessionLog, registry::Registry};
        let sandbox = tempfile::tempdir().unwrap();
        let mut sessions = Sessions::open(sandbox.path(), 1).unwrap();
        let owner = sessions.create(Default::default()).unwrap();
        let source = sessions.create(Default::default()).unwrap();
        drop(sessions);
        // A crash after the log append leaves the trigger and the rename out of the index.
        let log = SessionLog::new(sandbox.path(), owner.id, owner.device_id);
        let epoch = log.read().unwrap().entries.last().unwrap().epoch;
        let id = ws_core::ids::Ulid::new();
        log.append(
            epoch,
            "edge.action_registered",
            json!({
                "id":id, "owner":owner.id, "sources":[source.id], "event":"done",
                "action":"send", "target":owner.id, "template":"Continue {id}",
                "agent":null, "workspace":null, "spawned_session":null,
                "event_offsets":{}, "edges":[]
            }),
        )
        .unwrap();
        log.append(epoch, "session.renamed", json!({"name":"late"}))
            .unwrap();
        let restored = |sessions: &Sessions| {
            (
                sessions.registry.sessions().unwrap(),
                sessions.triggers.keys().copied().collect::<Vec<_>>(),
            )
        };
        let caught_up = restored(&Sessions::open(sandbox.path(), 1).unwrap());
        assert!(
            caught_up
                .0
                .iter()
                .any(|s| s.name.as_deref() == Some("late"))
        );
        assert_eq!(caught_up.1, vec![id]);
        // An index row without a log forces the complete rebuild.
        let elsewhere = tempfile::tempdir().unwrap();
        let orphan = SessionLog::new(elsewhere.path(), ws_core::ids::Ulid::new(), owner.device_id);
        Registry::open(&sandbox.path().join("registry.sqlite"), 1)
            .unwrap()
            .apply(
                &orphan
                    .append(0, "session.created", json!({"alias": 99}))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            restored(&Sessions::open(sandbox.path(), 1).unwrap()),
            caught_up
        );
    }

    #[tokio::test]
    async fn immediate_action_failures_return_an_error_and_never_repeat() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, source) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, target) = request(&app, "POST", "/v1/sessions", json!({})).await;
        request(
            &app,
            "POST",
            &format!("/v1/sessions/{}/done", source["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        let output = sandbox
            .path()
            .join("sessions")
            .join(source["id"].as_str().unwrap())
            .join("output.md");
        std::fs::remove_file(&output).unwrap();
        std::fs::create_dir(&output).unwrap();
        let (status, error) = request(&app, "POST", "/v1/triggers", json!({"sources":[source["id"]], "event":"done", "target":target["id"], "template":"next"})).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
        assert_eq!(
            request(&app, "GET", "/v1/triggers", Value::Null).await.1,
            json!([])
        );
        assert_eq!(
            request(&app, "GET", "/v1/edges", Value::Null).await.1,
            json!([])
        );
    }

    #[tokio::test]
    async fn exit_watchers_ignore_old_exits_and_keep_fan_in_across_restarts() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, a) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, b) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, owner) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let exit = |id: &Value, observed| json!({"session_id":id, "callback_id":ws_core::ids::Ulid::new(), "observed_at_us":observed, "state":"exit", "pane":null, "payload":{"hook_event_name":"SessionEnd", "exit_code":0}});
        assert_eq!(
            request(&app, "POST", "/v1/hooks/ingest", exit(&a["id"], 1))
                .await
                .0,
            StatusCode::CREATED
        );
        let (status, trigger) = request(&app, "POST", "/v1/triggers", json!({"sources":[a["id"], b["id"]], "event":"exit", "target":owner["id"], "template":"after {id}", "parent_session_id":owner["id"]})).await;
        assert_eq!(status, StatusCode::CREATED, "{trigger}");
        assert_eq!(
            request(&app, "POST", "/v1/hooks/ingest", exit(&b["id"], 2))
                .await
                .0,
            StatusCode::CREATED
        );
        assert_eq!(
            request(&app, "GET", "/v1/triggers", Value::Null)
                .await
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
        drop(app);
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        assert_eq!(
            request(&app, "GET", "/v1/triggers", Value::Null)
                .await
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
        // A new launch allows a new hook exit after the old launch ended.
        // The API permits an independent callback while the source is still created.
        assert_eq!(
            request(&app, "POST", "/v1/hooks/ingest", exit(&a["id"], 3))
                .await
                .0,
            StatusCode::CREATED
        );
        let pending = request(&app, "GET", "/v1/triggers", Value::Null).await.1;
        assert_eq!(pending, json!([]));
        let inbox = request(
            &app,
            "GET",
            &format!("/v1/inbox?session_id={}", owner["id"].as_str().unwrap()),
            Value::Null,
        )
        .await
        .1;
        assert_eq!(inbox.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cycles_include_dependencies_and_removal_releases_the_graph() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(request(&app, "POST", "/v1/sessions", json!({})).await.1["id"].clone());
        }
        let registration = |a: usize, b: usize, event| json!({"sources":[ids[a]], "target":ids[b], "event":event, "template":"next"});
        let (status, first) =
            request(&app, "POST", "/v1/triggers", registration(0, 1, "exit")).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            request(&app, "POST", "/v1/triggers", registration(1, 0, "exit"))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(&app, "POST", "/v1/triggers", registration(2, 2, "done"))
                .await
                .0,
            StatusCode::CONFLICT
        );
        let (status, second) =
            request(&app, "POST", "/v1/triggers", registration(1, 2, "exit")).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            request(&app, "POST", "/v1/triggers", registration(2, 0, "exit"))
                .await
                .0,
            StatusCode::CONFLICT
        );
        let (_, edges) = request(&app, "GET", "/v1/edges", Value::Null).await;
        assert_eq!(edges.as_array().unwrap().len(), 2);
        for trigger in [first, second] {
            assert_eq!(
                request(
                    &app,
                    "POST",
                    &format!("/v1/triggers/{}/remove", trigger["id"].as_str().unwrap()),
                    json!({})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        assert_eq!(
            request(&app, "GET", "/v1/edges", Value::Null).await.1,
            json!([])
        );
        let endpoint = format!("/v1/sessions/{}/depends_on", ids[0].as_str().unwrap());
        assert_eq!(
            request(&app, "POST", &endpoint, json!({"target":ids[1]}))
                .await
                .0,
            StatusCode::CREATED
        );
        assert_eq!(
            request(&app, "POST", "/v1/triggers", registration(0, 1, "done"))
                .await
                .0,
            StatusCode::CONFLICT
        );
        // A one-shot crash handler may send to its exited source.
        assert_eq!(
            request(&app, "POST", "/v1/triggers", registration(2, 2, "exit"))
                .await
                .0,
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn spawn_requires_an_existing_workspace_before_registration() {
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, source) = request(&app, "POST", "/v1/sessions", json!({})).await;
        request(
            &app,
            "POST",
            &format!("/v1/sessions/{}/done", source["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
        // A missing workspace must fail at registration, before any action is stored.
        let (status, _) = request(
            &app,
            "POST",
            "/v1/triggers",
            json!({"sources":[source["id"]], "event":"done", "spawn":"claude", "template":"start"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, registered) = request(&app, "POST", "/v1/triggers", json!({"sources":[source["id"]], "event":"done", "spawn":"claude", "workspace":"missing", "template":"start"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{registered}");
    }

    #[tokio::test]
    async fn fan_in_sends_once_from_the_owner_and_preserves_literal_output() {
        // Golden scenario: ws/tests/triggers.test.sh.
        let sandbox = tempfile::tempdir().unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        let (_, a) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, b) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (_, owner) = request(&app, "POST", "/v1/sessions", json!({})).await;
        let (status, trigger) = request(
            &app,
            "POST",
            "/v1/triggers",
            json!({
                "sources":[a["id"], b["id"], a["id"]], "event":"done", "target":owner["id"],
                "template":"results {id}: {output}", "parent_session_id":owner["id"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{trigger}");
        assert_eq!(trigger["sources"], json!([a["id"], b["id"]]));
        let endpoint = format!("/v1/inbox?session_id={}", owner["id"].as_str().unwrap());
        let (status, _) = request(
            &app,
            "POST",
            &format!("/v1/sessions/{}/done", a["id"].as_str().unwrap()),
            json!({"output":"alpha \"quoted\" $literal {id}"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, inbox) = request(&app, "GET", &endpoint, Value::Null).await;
        assert_eq!(inbox, json!([]));
        let done = format!("/v1/sessions/{}/done", b["id"].as_str().unwrap());
        let repeat = format!("/v1/sessions/{}/done", a["id"].as_str().unwrap());
        let (first, second) = tokio::join!(
            request(&app, "POST", &done, json!({"output":"beta\n"})),
            request(
                &app,
                "POST",
                &repeat,
                json!({"output":"alpha \"quoted\" $literal {id}"})
            )
        );
        assert_eq!(first.0, StatusCode::OK);
        assert_eq!(second.0, StatusCode::OK);
        for _ in 0..2 {
            assert_eq!(
                request(&app, "POST", &done, json!({"output":"beta\n"}))
                    .await
                    .0,
                StatusCode::OK
            );
        }
        let (_, inbox) = request(&app, "GET", &endpoint, Value::Null).await;
        assert_eq!(inbox.as_array().unwrap().len(), 1);
        assert_eq!(inbox[0]["from_session"], owner["id"]);
        assert_eq!(
            inbox[0]["body"],
            format!(
                "results {},{}: alpha \"quoted\" $literal {{id}}\nbeta\n",
                a["id"].as_str().unwrap(),
                b["id"].as_str().unwrap()
            )
        );
        let (_, pending) = request(&app, "GET", "/v1/triggers", Value::Null).await;
        assert_eq!(pending, json!([]));
        drop(app);
        std::fs::remove_file(sandbox.path().join("registry.sqlite")).unwrap();
        let app = crate::api::router(Sessions::open(sandbox.path(), 1).unwrap());
        assert_eq!(
            request(&app, "GET", "/v1/triggers", Value::Null).await.1,
            json!([])
        );
        assert_eq!(
            request(&app, "GET", &endpoint, Value::Null)
                .await
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
