use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_import_profiles".to_string(),
            description: "List saved CSV import profiles (column mappings per institution)"
                .to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        false,
        |db, _params| async move {
            match aequi_storage::get_import_profiles(&db).await {
                Ok(profiles) => ToolResult::text(serde_json::to_string_pretty(&profiles).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_save_import_profile".to_string(),
            description: "Save a CSV import profile with column mappings".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Profile name (e.g., 'Chase Checking')" },
                    "has_header": { "type": "boolean", "default": true },
                    "delimiter": { "type": "string", "default": "," },
                    "date_column": { "type": "integer", "description": "0-based column index for date" },
                    "description_column": { "type": "integer", "description": "0-based column index for description" },
                    "amount_column": { "type": "integer", "description": "0-based column index for amount (single column)" },
                    "debit_column": { "type": "integer", "description": "0-based column index for debit (separate columns)" },
                    "credit_column": { "type": "integer", "description": "0-based column index for credit (separate columns)" },
                    "memo_column": { "type": "integer", "description": "0-based column index for memo" },
                    "date_format": { "type": "string", "default": "%m/%d/%Y" }
                },
                "required": ["name"]
            }),
        },
        true,
        |db, params| async move {
            let profile = aequi_storage::ImportProfile {
                id: 0,
                name: params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                has_header: params
                    .get("has_header")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                delimiter: params
                    .get("delimiter")
                    .and_then(|v| v.as_str())
                    .unwrap_or(",")
                    .to_string(),
                date_column: params.get("date_column").and_then(|v| v.as_i64()),
                description_column: params.get("description_column").and_then(|v| v.as_i64()),
                amount_column: params.get("amount_column").and_then(|v| v.as_i64()),
                debit_column: params.get("debit_column").and_then(|v| v.as_i64()),
                credit_column: params.get("credit_column").and_then(|v| v.as_i64()),
                memo_column: params.get("memo_column").and_then(|v| v.as_i64()),
                date_format: params
                    .get("date_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("%m/%d/%Y")
                    .to_string(),
                created_at: String::new(),
            };

            match aequi_storage::save_import_profile(&db, &profile).await {
                Ok(id) => ToolResult::text(json!({ "profile_id": id }).to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_pending_imports".to_string(),
            description: "List imported transactions pending review for a batch".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "batch_id": { "type": "string", "description": "Import batch ID" }
                },
                "required": ["batch_id"]
            }),
        },
        false,
        |db, params| async move {
            let batch_id = params
                .get("batch_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match aequi_storage::get_imported_transactions_for_review(&db, batch_id).await {
                Ok(rows) => ToolResult::text(serde_json::to_string_pretty(&rows).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
