use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, check_date_order, not_found};
use crate::tools::{Access, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_create_reconciliation_session".to_string(),
            description: "Create a reconciliation session for an account".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": { "type": "integer" },
                    "start_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "statement_balance_cents": { "type": "integer" }
                },
                "required": ["account_id", "start_date", "end_date", "statement_balance_cents"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let account_id = args.id("account_id")?;
            let start = args.date("start_date")?;
            let end = args.date("end_date")?;
            check_date_order(start, end)?;
            let balance = args.cents("statement_balance_cents")?;

            let exists = sqlx::query_as::<_, (i64,)>("SELECT id FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_optional(&db)
                .await?;
            if exists.is_none() {
                return Err(ToolError(format!("Account {account_id} not found")));
            }

            let id =
                aequi_storage::create_reconciliation_session(&db, account_id, start, end, balance)
                    .await?;
            Ok(ToolResult::text(json!({ "session_id": id }).to_string()))
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
        Access::Read,
        |db, params| async move {
            let session_id = Args::new(&params).id("session_id")?;
            let exists =
                sqlx::query_as::<_, (i64,)>("SELECT id FROM reconciliation_sessions WHERE id = ?")
                    .bind(session_id)
                    .fetch_optional(&db)
                    .await?;
            if exists.is_none() {
                return Err(ToolError(format!(
                    "Reconciliation session {session_id} not found"
                )));
            }
            let items = aequi_storage::get_reconciliation_items(&db, session_id).await?;
            Ok(ToolResult::json(&items))
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
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let item_id = args.id("item_id")?;
            let notes = args.str("notes")?;

            aequi_storage::resolve_reconciliation_item(&db, item_id, notes)
                .await
                .map_err(not_found(format!("Reconciliation item {item_id}")))?;
            Ok(ToolResult::text("Item resolved".to_string()))
        },
    );
}
