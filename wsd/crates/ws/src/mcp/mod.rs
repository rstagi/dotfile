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

use params::{Create, Dependency, Empty, Loop, Send, Spawn, Stream, Tail, Target, Wait};
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
        Ok(Self {
            socket: socket.into(),
            caller,
        })
    }

    pub fn with_caller(mut self, caller: Option<SessionId>) -> Self {
        self.caller = caller;
        self
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
                let body = if request.headless {
                    if request.name.is_some() || request.workspace_id.is_some() {
                        return Err(
                            "Headless creation does not accept name or workspace_id.".into()
                        );
                    }
                    let directory = std::env::current_dir()?;
                    serde_json::to_value(ws_daemon::supervisor::CreateHeadless {
                        headless: true,
                        command: request.command,
                        cwd: directory.join(request.cwd.unwrap_or_else(|| directory.clone())),
                        parent_session_id: self.caller,
                        env: request.env,
                        stdin: request.stdin.map(|path| directory.join(path)),
                        stdout: request.stdout.map(|path| directory.join(path)),
                        stderr: request.stderr.map(|path| directory.join(path)),
                    })?
                } else {
                    if !request.command.is_empty()
                        || request.cwd.is_some()
                        || !request.env.is_empty()
                        || request.stdin.is_some()
                        || request.stdout.is_some()
                        || request.stderr.is_some()
                    {
                        return Err("Set headless to true to create a process.".into());
                    }
                    json!({"name":request.name,"workspace_id":request.workspace_id})
                };
                self.request(Method::POST, "/v1/sessions", Some(body)).await
            }
            _ => Err(format!("Unknown tool: {name}.").into()),
        }
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
            "Create a registered session or a headless process.",
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
