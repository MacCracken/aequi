use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError};
use crate::tools::{Access, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_accounts".to_string(),
            description: "List all active accounts in the chart of accounts".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let accounts = aequi_storage::get_all_accounts(&db).await?;
            Ok(ToolResult::json(&accounts))
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
        Access::Read,
        |db, params| async move {
            let code = Args::new(&params).str("code")?;
            match aequi_storage::get_account_by_code(&db, code).await? {
                Some(account) => Ok(ToolResult::json(&account)),
                None => Err(ToolError(format!("Account {code} not found"))),
            }
        },
    );
}
