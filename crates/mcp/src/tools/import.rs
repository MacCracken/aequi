use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError};
use crate::tools::{Access, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_import_profiles".to_string(),
            description: "List saved CSV import profiles (column mappings per institution)"
                .to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let profiles = aequi_storage::get_import_profiles(&db).await?;
            Ok(ToolResult::json(&profiles))
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
                    "delimiter": { "type": "string", "default": ",", "description": "A single character" },
                    "date_column": { "type": "integer", "minimum": 0, "description": "0-based column index for date" },
                    "description_column": { "type": "integer", "minimum": 0, "description": "0-based column index for description" },
                    "amount_column": { "type": "integer", "minimum": 0, "description": "0-based column index for amount (single column)" },
                    "debit_column": { "type": "integer", "minimum": 0, "description": "0-based column index for debit (separate columns)" },
                    "credit_column": { "type": "integer", "minimum": 0, "description": "0-based column index for credit (separate columns)" },
                    "memo_column": { "type": "integer", "minimum": 0, "description": "0-based column index for memo" },
                    "date_format": { "type": "string", "default": "%m/%d/%Y" }
                },
                "required": ["name"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let column = |name: &str| -> Result<Option<i64>, ToolError> {
                match args.opt_i64(name)? {
                    Some(c) if c < 0 => Err(ToolError(format!(
                        "Argument {name} must be a column index of 0 or more, got {c}"
                    ))),
                    other => Ok(other),
                }
            };

            // The delimiter is matched literally, so a blank one (e.g. a tab)
            // is allowed; it just has to be a single character.
            let delimiter = match params.get("delimiter") {
                None | Some(serde_json::Value::Null) => ",".to_string(),
                Some(serde_json::Value::String(d)) if d.chars().count() == 1 => d.clone(),
                Some(other) => {
                    return Err(ToolError(format!(
                        "Argument delimiter must be a single character, got {other}"
                    )));
                }
            };

            let profile = aequi_storage::ImportProfile {
                id: 0,
                name: args.str("name")?.to_string(),
                has_header: args.opt_bool("has_header")?.unwrap_or(true),
                delimiter,
                date_column: column("date_column")?,
                description_column: column("description_column")?,
                amount_column: column("amount_column")?,
                debit_column: column("debit_column")?,
                credit_column: column("credit_column")?,
                memo_column: column("memo_column")?,
                date_format: args
                    .opt_str("date_format")?
                    .unwrap_or("%m/%d/%Y")
                    .to_string(),
                created_at: String::new(),
            };

            let id = aequi_storage::save_import_profile(&db, &profile).await?;
            Ok(ToolResult::text(json!({ "profile_id": id }).to_string()))
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
        Access::Read,
        |db, params| async move {
            let batch_id = Args::new(&params).str("batch_id")?;
            let rows = aequi_storage::get_imported_transactions_for_review(&db, batch_id).await?;
            Ok(ToolResult::json(&rows))
        },
    );
}
