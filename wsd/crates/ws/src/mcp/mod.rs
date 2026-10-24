use std::path::{Path, PathBuf};

use hyper::Method;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use ws_core::ids::SessionId;

use crate::{Result, client};

mod params;

use params::{
    Agent, Create, Dependency, Empty, Loop, Rename, Send, Spawn, Stream, Tail, Target, Wait,
};
use rmcp::schemars::{self, JsonSchema};

#[derive(Clone)]
pub struct Control {
    socket: PathBuf,
    caller: Option<SessionId>,
}

impl Control {
    pub fn from_env(socket: &Path) -> Result<Self> {
        let caller = std::env::var("WS_SESSION_ID")
            .ok()
            .map(|id| id.parse())
            .transpose()?;
        Ok(Self::for_caller(socket, caller))
    }

    pub fn for_caller(socket: &Path, caller: Option<SessionId>) -> Self {
        Self {
            socket: socket.into(),
            caller,
        }
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        match name {
            "loop_start" | "loop_stop" => {
                let mut request: Loop = serde_json::from_value(arguments)?;
                request.loop_dir = std::env::current_dir()?.join(request.loop_dir);
                let operation = name.strip_prefix("loop_").unwrap();
                self.request(
                    Method::POST,
                    &format!("/v1/loops/{operation}"),
                    Some(serde_json::to_value(request)?),
                )
                .await
            }
            "session_list" => {
                let _: Empty = serde_json::from_value(arguments)?;
                self.request(Method::GET, "/v1/sessions", None).await
            }
            "session_get" => {
                let request: Target = serde_json::from_value(arguments)?;
                self.request(Method::GET, &self.session_path(&request.target)?, None)
                    .await
            }
            "session_rename" => {
                let request: Rename = serde_json::from_value(arguments)?;
                let caller = self
                    .caller
                    .ok_or("Set WS_SESSION_ID to rename your session.")?;
                self.request(
                    Method::POST,
                    &format!("{}/name", self.session_path(&caller.to_string())?),
                    Some(json!({"name":request.name})),
                )
                .await
            }
            "session_depends_on" | "session_block_on" => {
                let request: Dependency = serde_json::from_value(arguments)?;
                let operation = name.strip_prefix("session_").unwrap();
                self.request(
                    Method::POST,
                    &format!("{}/{operation}", self.session_path(&request.target)?),
                    Some(json!({"target":request.dependency})),
                )
                .await
            }
            "phase_spawn" => {
                self.caller.ok_or("Set WS_SESSION_ID to spawn a phase.")?;
                let mut request: Spawn = serde_json::from_value(arguments)?;
                request.repo = std::env::current_dir()?.join(request.repo);
                self.request(
                    Method::POST,
                    "/v1/phases/spawn",
                    Some(serde_json::to_value(request)?),
                )
                .await
            }
            "session_tail" => {
                let request: Tail = serde_json::from_value(arguments)?;
                let stream = match request.stream.unwrap_or(Stream::Stdout) {
                    Stream::Stdout => "stdout",
                    Stream::Stderr => "stderr",
                };
                let max_bytes = request
                    .max_bytes
                    .unwrap_or(ws_daemon::tail::DEFAULT_MAX_BYTES);
                let path = format!(
                    "{}/tail?stream={stream}&max_bytes={max_bytes}",
                    self.session_path(&request.target)?
                );
                self.request(Method::GET, &path, None).await
            }
            "wait" => {
                let request: Wait = serde_json::from_value(arguments)?;
                self.request(
                    Method::POST,
                    &format!("{}/wait", self.session_path(&request.target)?),
                    Some(json!({"for":request.condition, "timeout":request.timeout})),
                )
                .await
            }
            "session_send" => {
                let request: Send = serde_json::from_value(arguments)?;
                self.request(
                    Method::POST,
                    "/v1/messages",
                    Some(serde_json::to_value(request)?),
                )
                .await
            }
            "session_stop" => {
                let request: Target = serde_json::from_value(arguments)?;
                self.request(
                    Method::POST,
                    &format!("{}/stop", self.session_path(&request.target)?),
                    None,
                )
                .await
            }
            "session_create" => {
                let request: Create = serde_json::from_value(arguments)?;
                if request.headless {
                    return self.create_headless(request).await;
                }
                if !request.command.is_empty()
                    || !request.env.is_empty()
                    || request.stdin.is_some()
                    || request.stdout.is_some()
                    || request.stderr.is_some()
                {
                    return Err("Set headless to true to create a process.".into());
                }
                let Some(place) = start_place(&request)? else {
                    if request.agent.is_some() || request.prompt.is_some() {
                        return Err("Set cwd, scratch or repo to start an agent.".into());
                    }
                    return self
                        .request(
                            Method::POST,
                            "/v1/sessions",
                            Some(json!({"name":request.name,"workspace_id":request.workspace_id})),
                        )
                        .await;
                };
                let agent = match request.agent.unwrap_or_default() {
                    Agent::Claude => "claude",
                    Agent::Codex => "codex",
                    Agent::Grok => "grok",
                };
                let (path, body) = crate::cmd::new::request(
                    place,
                    crate::cmd::new::Launch {
                        agent: agent.into(),
                        name: request.name,
                        prompt: request.prompt,
                    },
                    |_| {
                        Err("Set branch to create a worktree from a git repository. Use cwd to work in the checkout.".into())
                    },
                )?;
                let launched = self.request(Method::POST, path, Some(body)).await?;
                let id = launched["session_id"]
                    .as_str()
                    .ok_or("The daemon did not return a session id.")?;
                self.request(Method::GET, &self.session_path(id)?, None)
                    .await
            }
            _ => Err(format!("Unknown tool: {name}.").into()),
        }
    }

    async fn create_headless(&self, request: Create) -> Result<Value> {
        if request.name.is_some()
            || request.workspace_id.is_some()
            || request.scratch
            || request.repo.is_some()
            || request.branch.is_some()
            || request.agent.is_some()
            || request.prompt.is_some()
        {
            return Err(
                "Headless creation does not accept name, workspace_id, scratch, repo, branch, agent or prompt.".into(),
            );
        }
        let directory = std::env::current_dir()?;
        let body = serde_json::to_value(ws_daemon::supervisor::CreateHeadless {
            headless: true,
            command: request.command,
            cwd: directory.join(request.cwd.unwrap_or_else(|| directory.clone())),
            parent_session_id: self.caller,
            env: request.env,
            stdin: request.stdin.map(|path| directory.join(path)),
            stdout: request.stdout.map(|path| directory.join(path)),
            stderr: request.stderr.map(|path| directory.join(path)),
        })?;
        self.request(Method::POST, "/v1/sessions", Some(body)).await
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (path, body) = if method == Method::GET {
            (self.query(path), Vec::new())
        } else {
            let mut body = body.unwrap_or(json!({}));
            body["parent_session_id"] = json!(self.caller);
            (path.into(), serde_json::to_vec(&body)?)
        };
        let body = client::request(&self.socket, method, &path, body).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    fn session_path(&self, target: &str) -> Result<String> {
        let target = if target == "me" {
            self.caller
                .ok_or("Set WS_SESSION_ID to use me.")?
                .to_string()
        } else {
            target.to_owned()
        };
        Ok(format!("/v1/sessions/{}", client::encode(&target)))
    }

    fn query(&self, path: &str) -> String {
        match self.caller {
            Some(caller) => format!(
                "{path}{}parent_session_id={caller}",
                if path.contains('?') { "&" } else { "?" }
            ),
            None => path.into(),
        }
    }
}

impl ServerHandler for Control {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ws", env!("CARGO_PKG_VERSION")))
            .with_instructions("Tools use the local daemon. WS_SESSION_ID identifies the caller. Tool arguments cannot replace this identity.")
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<rmcp::model::CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params("The tool name is unknown.", None));
        }
        let result = self
            .call(
                &request.name,
                Value::Object(request.arguments.unwrap_or_default()),
            )
            .await;
        Ok(match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(client::error_json(error.as_ref())),
        }
        .into())
    }
}

fn tools() -> Vec<Tool> {
    vec![
        tool::<Loop>(
            "loop_start",
            "Start a prepared Loop orchestrator as a headless session. The loop directory must contain state.json.",
        ),
        tool::<Loop>(
            "loop_stop",
            "Pause a loop through its control/pause file. The orchestrator and phase runners observe this file.",
        ),
        tool::<Empty>("session_list", "List the registered sessions."),
        tool::<Target>("session_get", "Read one registered session."),
        tool::<Target>(
            "session_stop",
            "Stop a session and wait for its process group to exit.",
        ),
        tool::<Create>(
            "session_create",
            "Create a registered session or a headless process. Set cwd, scratch or repo to start an interactive agent. Set repo and branch to create a git worktree and a tmux workspace. Your session becomes the parent. The result is the new session record.",
        ),
        tool::<Rename>(
            "session_rename",
            "Rename your own session. ws names each session from its first prompt. Use this tool only when that name is wrong. Use 2 to 4 words in kebab case. Names are unique.",
        ),
        tool::<Send>("session_send", "Queue a message for a session."),
        tool::<Wait>("wait", "Wait for a session state or event."),
        tool::<Spawn>(
            "phase_spawn",
            "Start a local phase with a maximum runtime. WS_SESSION_ID identifies its parent.",
        ),
        tool::<Tail>(
            "session_tail",
            "Read the last output bytes from a session. The maximum is 65536 bytes.",
        ),
        tool::<Dependency>(
            "session_depends_on",
            "Add a dependency between two sessions.",
        ),
        tool::<Dependency>("session_block_on", "Block a session on another session."),
    ]
}

fn tool<T: JsonSchema>(name: &'static str, description: &'static str) -> Tool {
    let schema = schemars::schema_for!(T);
    Tool::new(
        name,
        description,
        schema
            .as_object()
            .expect("Tool parameters must have an object schema.")
            .clone(),
    )
}

/// Returns the interactive start place, or None for a record without an agent.
fn start_place(request: &Create) -> Result<Option<crate::cmd::new::Place>> {
    use crate::cmd::new::Place;
    let places = [
        request.cwd.is_some(),
        request.scratch,
        request.repo.is_some(),
        request.workspace_id.is_some(),
    ];
    if places.into_iter().filter(|set| *set).count() > 1 {
        return Err("Use only one of cwd, scratch, repo or workspace_id.".into());
    }
    if request.branch.is_some() && request.repo.is_none() {
        return Err("Set repo to use branch.".into());
    }
    let directory = std::env::current_dir()?;
    Ok(if let Some(cwd) = &request.cwd {
        Some(Place::Directory(directory.join(cwd)))
    } else if request.scratch {
        Some(Place::Scratch)
    } else {
        request.repo.as_ref().map(|repo| Place::Repo {
            repo: directory.join(repo),
            branch: request.branch.clone(),
        })
    })
}
