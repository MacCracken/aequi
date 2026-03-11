use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_accounts".to_string(),
            description: "List all active accounts in the chart of accounts".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        false,
        |db, _params| async move {
            match aequi_storage::get_all_accounts(&db).await {
                Ok(accounts) => ToolResult::text(serde_json::to_string_pretty(&accounts).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_account".to_string(),
            description: "Get an account by its code".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "code": { "type": "string" } },
                "required": ["code"]
            }),
        },
        false,
        |db, params| async move {
            let code = params.get("code").and_then(|v| v.as_str()).unwrap_or("");
            match aequi_storage::get_account_by_code(&db, code).await {
                Ok(Some(account)) => {
                    ToolResult::text(serde_json::to_string_pretty(&account).unwrap())
                }
                Ok(None) => ToolResult::error(format!("Account {code} not found")),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
