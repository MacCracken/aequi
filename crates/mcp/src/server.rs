use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::audit;
use crate::permissions::Permissions;
use crate::protocol::{
    INTERNAL_ERROR, INVALID_PARAMS, JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND,
    parse_message,
};
use crate::tools::ToolRegistry;

pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Transport-independent MCP request handling, shared by stdio and SSE.
pub struct McpServer {
    db: aequi_storage::DbPool,
    permissions: Permissions,
    registry: ToolRegistry,
}

impl McpServer {
    pub fn new(db: aequi_storage::DbPool, permissions: Permissions) -> Self {
        McpServer {
            db,
            permissions,
            registry: ToolRegistry::new(),
        }
    }

    /// Handle one raw JSON-RPC message. Returns `None` when nothing may be
    /// sent back, i.e. for notifications.
    pub async fn handle_message(&self, bytes: &[u8]) -> Option<JsonRpcResponse> {
        match parse_message(bytes) {
            Ok(req) => self.handle_request(&req).await,
            Err(error_response) => Some(error_response),
        }
    }

    pub async fn handle_request(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        let Some(id) = req.id.clone() else {
            // Notifications (notifications/initialized, notifications/cancelled,
            // ...) need no action here and must never be answered.
            tracing::debug!("ignoring notification {}", req.method);
            return None;
        };

        Some(match req.method.as_str() {
            "initialize" => JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": { "listChanged": false },
                    },
                    "serverInfo": {
                        "name": "aequi-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "ping" => JsonRpcResponse::success(id, json!({})),
            "tools/list" => {
                let tools: Vec<_> = self
                    .registry
                    .list_definitions()
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema
                        })
                    })
                    .collect();
                JsonRpcResponse::success(id, json!({ "tools": tools }))
            }
            "tools/call" => self.call_tool(id, &req.params).await,
            _ => JsonRpcResponse::error(
                id,
                METHOD_NOT_FOUND,
                format!("Method not found: {}", req.method),
            ),
        })
    }

    async fn call_tool(&self, id: Value, params: &Value) -> JsonRpcResponse {
        let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                "Invalid params: tools/call requires a string \"name\"".to_string(),
            );
        };
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

        let input_str = arguments.to_string();
        let result = self
            .registry
            .call(tool_name, arguments, &self.db, &self.permissions)
            .await;

        let outcome = if result.is_error() {
            "error"
        } else {
            "success"
        };
        audit::log_tool_call(&self.db, tool_name, &input_str, outcome).await;

        match serde_json::to_value(&result) {
            Ok(value) => JsonRpcResponse::success(id, value),
            Err(e) => JsonRpcResponse::error(
                id,
                INTERNAL_ERROR,
                format!("Failed to serialize tool result: {e}"),
            ),
        }
    }
}

pub async fn run_stdio_server(db: aequi_storage::DbPool, permissions: Permissions) -> Result<()> {
    let server = McpServer::new(db, permissions);
    serve_lines(
        &server,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

/// Newline-delimited JSON-RPC over a byte stream. Only protocol messages are
/// written to `writer`; logging goes to stderr. A malformed line (including
/// invalid UTF-8) gets a parse error and the loop keeps going.
pub async fn serve_lines<R, W>(server: &McpServer, mut reader: R, mut writer: W) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).await? == 0 {
            break; // EOF
        }

        let line = buf.trim_ascii();
        if line.is_empty() {
            continue;
        }

        if let Some(response) = server.handle_message(line).await {
            let mut out = serde_json::to_vec(&response)?;
            out.push(b'\n');
            writer.write_all(&out).await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
