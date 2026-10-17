use std::path::{Path, PathBuf};

use hyper::Method;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use ws_core::ids::SessionId;

use crate::{Result, client};

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

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        if name != "session_list" {
            return Err(format!("Unknown tool: {name}.").into());
        }
        if arguments != json!({}) {
            return Err("The session_list tool accepts no arguments.".into());
        }
        let path = self.query("/v1/sessions");
        let body = client::request(&self.socket, Method::GET, &path, Vec::new()).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    fn query(&self, path: &str) -> String {
        match self.caller {
            Some(caller) => format!("{path}?parent_session_id={caller}"),
            None => path.into(),
        }
    }
}

impl ServerHandler for Control {
    fn get_info(&self) -> ServerConfig {
        {
            let mut info = ServerConfig::default();
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info
        }
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        {
            let mut result = ListToolsResult::default();
            result.tools = tools();
            Ok(result)
        }
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
            Err(error) => {
                CallToolResult::structured_error(json!({"error": {"message": error.to_string()}}))
            }
        }
        .into())
    }
}

fn tools() -> Vec<Tool> {
    vec![Tool::new(
        "session_list",
        "List the registered sessions.",
        json!({"type":"object", "properties":{}, "additionalProperties":false})
            .as_object()
            .unwrap()
            .clone(),
    )]
}
