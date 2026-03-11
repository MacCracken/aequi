use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_create_reconciliation_session".to_string(),
            description: "Create a reconciliation session for an account".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": { "type": "integer" },
                    "start_date": { "type": "string" },
                    "end_date": { "type": "string" },
                    "statement_balance_cents": { "type": "integer" }
                },
                "required": ["account_id", "start_date", "end_date", "statement_balance_cents"]
            }),
        },
        true,
        |db, params| async move {
            let account_id = params
                .get("account_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let start = params
                .get("start_date")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let end = params
                .get("end_date")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let balance = params
                .get("statement_balance_cents")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            match aequi_storage::create_reconciliation_session(&db, account_id, start, end, balance)
                .await
            {
                Ok(id) => ToolResult::text(json!({ "session_id": id }).to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_reconciliation_items".to_string(),
            description: "Get items in a reconciliation session".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "session_id": { "type": "integer" } },
                "required": ["session_id"]
            }),
        },
        false,
        |db, params| async move {
            let session_id = params
                .get("session_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            match aequi_storage::get_reconciliation_items(&db, session_id).await {
                Ok(items) => ToolResult::text(serde_json::to_string_pretty(&items).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_resolve_item".to_string(),
            description: "Resolve a reconciliation item".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "item_id": { "type": "integer" },
                    "notes": { "type": "string" }
                },
                "required": ["item_id", "notes"]
            }),
        },
        true,
        |db, params| async move {
            let item_id = params.get("item_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let notes = params.get("notes").and_then(|v| v.as_str()).unwrap_or("");

            match aequi_storage::resolve_reconciliation_item(&db, item_id, notes).await {
                Ok(()) => ToolResult::text("Item resolved".to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
