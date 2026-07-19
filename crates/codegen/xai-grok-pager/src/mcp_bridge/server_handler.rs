//! rmcp `ServerHandler` exposing Failure's session lifecycle as MCP tools.
//!
//! Mirrors the npm bridge's tool set (`npm/failure/bin/mcp-server.js`), minus
//! `failure_rpc`'s full generality: this bridge talks typed ACP requests
//! in-process, not raw JSON-RPC over a spawned subprocess's stdio, so
//! `failure_rpc` here is scoped to the `x.ai/*` extension-method namespace
//! rather than any ACP method by name.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde::Deserialize;

use super::agent::BridgeAgent;

const DEFAULT_TIMEOUT_MS: u64 = 30 * 60 * 1000;

#[derive(Clone)]
pub struct BridgeToolServer {
    agent: Arc<BridgeAgent>,
    default_cwd: PathBuf,
}

impl BridgeToolServer {
    pub fn new(agent: Arc<BridgeAgent>, default_cwd: PathBuf) -> Self {
        Self { agent, default_cwd }
    }
}

fn tool_schema(properties: serde_json::Value, required: &[&str]) -> rmcp::model::JsonObject {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
    .as_object()
    .cloned()
    .expect("tool schema is always a JSON object")
}

fn empty_schema() -> rmcp::model::JsonObject {
    serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false})
        .as_object()
        .cloned()
        .expect("empty schema is always a JSON object")
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "failure_new_chat",
            "Create a new persistent Failure chat in a workspace.",
            tool_schema(
                serde_json::json!({
                    "cwd": {"type": "string", "description": "Working directory (defaults to the bridge's own cwd)."},
                }),
                &[],
            ),
        ),
        Tool::new(
            "failure_continue_chat",
            "Load an existing Failure chat by session ID.",
            tool_schema(
                serde_json::json!({
                    "session_id": {"type": "string"},
                    "cwd": {"type": "string"},
                }),
                &["session_id"],
            ),
        ),
        Tool::new(
            "failure_send_message",
            "Send a message to a Failure chat. The agent can use its normal file, terminal, search, and coding tools.",
            tool_schema(
                serde_json::json!({
                    "session_id": {"type": "string"},
                    "message": {"type": "string"},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 3600000},
                }),
                &["session_id", "message"],
            ),
        ),
        Tool::new(
            "failure_list_sessions",
            "List saved Failure chats and sessions.",
            empty_schema(),
        ),
        Tool::new(
            "failure_status",
            "Show the remote bridge status.",
            empty_schema(),
        ),
        Tool::new(
            "failure_rpc",
            "Call a Failure `x.ai/*` extension method directly (e.g. `x.ai/session/list`, \
             `x.ai/session/rename`). Session lifecycle (new/continue/send message) has \
             dedicated tools above — use those instead of trying to reach `session/new`, \
             `session/load`, or `session/prompt` through here.",
            tool_schema(
                serde_json::json!({
                    "method": {"type": "string"},
                    "params": {"type": "object", "additionalProperties": true},
                }),
                &["method"],
            ),
        ),
    ]
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct NewChatArgs {
    cwd: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct ContinueChatArgs {
    session_id: String,
    cwd: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct SendMessageArgs {
    session_id: String,
    message: String,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
struct RpcArgs {
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

fn parse_args<T: for<'de> Deserialize<'de> + Default>(
    arguments: Option<rmcp::model::JsonObject>,
) -> Result<T, McpError> {
    match arguments {
        Some(map) => serde_json::from_value(serde_json::Value::Object(map))
            .map_err(|e| McpError::invalid_params(e.to_string(), None)),
        None => Ok(T::default()),
    }
}

fn text_result(value: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
    )])
}

fn error_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

impl ServerHandler for BridgeToolServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match request.name.as_ref() {
            "failure_new_chat" => {
                let args: NewChatArgs = parse_args(request.arguments)?;
                let cwd = args.cwd.map(PathBuf::from).unwrap_or_else(|| self.default_cwd.clone());
                match self.agent.new_chat(&cwd).await {
                    Ok(resp) => Ok(text_result(serde_json::json!({
                        "sessionId": resp.session_id.0,
                        "cwd": cwd,
                    }))),
                    Err(e) => Ok(error_result(e.to_string())),
                }
            }
            "failure_continue_chat" => {
                let args: ContinueChatArgs = parse_args(request.arguments)?;
                let cwd = args.cwd.map(PathBuf::from).unwrap_or_else(|| self.default_cwd.clone());
                match self.agent.continue_chat(&args.session_id, &cwd).await {
                    Ok(_resp) => Ok(text_result(serde_json::json!({
                        "sessionId": args.session_id,
                        "cwd": cwd,
                    }))),
                    Err(e) => Ok(error_result(e.to_string())),
                }
            }
            "failure_send_message" => {
                let args: SendMessageArgs = parse_args(request.arguments)?;
                let timeout =
                    Duration::from_millis(args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
                match self
                    .agent
                    .send_message(&args.session_id, &args.message, timeout)
                    .await
                {
                    Ok((response, text)) => Ok(text_result(serde_json::json!({
                        "stopReason": format!("{:?}", response.stop_reason),
                        "text": text,
                    }))),
                    Err(e) => Ok(error_result(e.to_string())),
                }
            }
            "failure_list_sessions" => {
                match self
                    .agent
                    .ext_rpc(
                        "x.ai/session/list",
                        serde_json::json!({ "cwd": self.default_cwd, "limit": 100 }),
                    )
                    .await
                {
                    Ok(value) => Ok(text_result(value)),
                    Err(e) => Ok(error_result(e.to_string())),
                }
            }
            "failure_status" => Ok(text_result(serde_json::json!({
                "ok": true,
                "agentRunning": self.agent.is_running(),
                "cwd": self.default_cwd,
            }))),
            "failure_rpc" => {
                let args: RpcArgs = parse_args(request.arguments)?;
                if !args.method.starts_with("x.ai/") {
                    return Ok(error_result(format!(
                        "failure_rpc only forwards `x.ai/*` extension methods in this bridge \
                         (got `{}`) — use failure_new_chat/failure_continue_chat/\
                         failure_send_message for session lifecycle.",
                        args.method
                    )));
                }
                match self.agent.ext_rpc(&args.method, args.params).await {
                    Ok(value) => Ok(text_result(value)),
                    Err(e) => Ok(error_result(e.to_string())),
                }
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}
