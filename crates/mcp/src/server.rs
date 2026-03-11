use anyhow::Result;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::audit;
use crate::permissions::Permissions;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::tools::ToolRegistry;

pub async fn run_stdio_server(db: aequi_storage::DbPool, permissions: Permissions) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    let registry = ToolRegistry::new();

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
                let out = serde_json::to_string(&resp)? + "\n";
                stdout.write_all(out.as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };

        let response = handle_request(&request, &registry, &db, &permissions).await;
        let out = serde_json::to_string(&response)? + "\n";
        stdout.write_all(out.as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_request(
    req: &JsonRpcRequest,
    registry: &ToolRegistry,
    db: &aequi_storage::DbPool,
    permissions: &Permissions,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => JsonRpcResponse::success(
            req.id.clone(),
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false },
                },
                "serverInfo": {
                    "name": "aequi-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "notifications/initialized" => {
            // No response needed for notifications
            JsonRpcResponse::success(req.id.clone(), json!({}))
        }
        "tools/list" => {
            let tools: Vec<_> = registry
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
            JsonRpcResponse::success(req.id.clone(), json!({ "tools": tools }))
        }
        "tools/call" => {
            let tool_name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

            let input_str = serde_json::to_string(&arguments).unwrap_or_default();
            let result = registry.call(tool_name, arguments, db, permissions).await;

            let outcome = if result.is_error.unwrap_or(false) {
                "error"
            } else {
                "success"
            };
            audit::log_tool_call(db, tool_name, &input_str, outcome).await;

            JsonRpcResponse::success(
                req.id.clone(),
                serde_json::to_value(&result).unwrap_or(json!({})),
            )
        }
        _ => JsonRpcResponse::error(
            req.id.clone(),
            -32601,
            format!("Method not found: {}", req.method),
        ),
    }
}
