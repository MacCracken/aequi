use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_get_categorization_rules".to_string(),
            description: "List all categorization rules".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        false,
        |db, _params| async move {
            match aequi_storage::get_categorization_rules(&db).await {
                Ok(rules) => ToolResult::text(serde_json::to_string_pretty(&rules).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_save_categorization_rule".to_string(),
            description: "Create a categorization rule".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "priority": { "type": "integer" },
                    "match_pattern": { "type": "string" },
                    "match_type": { "type": "string", "enum": ["contains", "exact", "regex", "fuzzy"] },
                    "account_id": { "type": "integer" }
                },
                "required": ["name", "priority", "match_pattern", "match_type", "account_id"]
            }),
        },
        true,
        |db, params| async move {
            let rule = aequi_storage::CategorizationRule {
                id: 0,
                name: params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                priority: params.get("priority").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                match_pattern: params.get("match_pattern").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                match_type: params.get("match_type").and_then(|v| v.as_str()).unwrap_or("contains").to_string(),
                account_id: params.get("account_id").and_then(|v| v.as_i64()).unwrap_or(0),
                created_at: String::new(),
            };

            match aequi_storage::save_categorization_rule(&db, &rule).await {
                Ok(id) => ToolResult::text(json!({ "rule_id": id }).to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_apply_rules".to_string(),
            description: "Apply categorization rules to uncategorized imported transactions in a batch"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "batch_id": { "type": "string", "description": "Import batch ID to apply rules to" }
                },
                "required": ["batch_id"]
            }),
        },
        true,
        |db, params| async move {
            let batch_id = params
                .get("batch_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let rules = match aequi_storage::get_categorization_rules(&db).await {
                Ok(r) => r,
                Err(e) => return ToolResult::error(e.to_string()),
            };
            let pending =
                match aequi_storage::get_pending_imported_transactions(&db, batch_id).await {
                    Ok(p) => p,
                    Err(e) => return ToolResult::error(e.to_string()),
                };

            let mut matched = 0i64;
            let total = pending.len();

            for tx in &pending {
                let desc_lower = tx.description.to_lowercase();
                for rule in &rules {
                    let is_match = match rule.match_type.as_str() {
                        "exact" => desc_lower == rule.match_pattern.to_lowercase(),
                        "contains" => desc_lower.contains(&rule.match_pattern.to_lowercase()),
                        _ => desc_lower.contains(&rule.match_pattern.to_lowercase()),
                    };
                    if is_match {
                        let _ = aequi_storage::mark_imported_transaction_categorized(
                            &db, tx.id, rule.id,
                        )
                        .await;
                        matched += 1;
                        break;
                    }
                }
            }

            ToolResult::text(
                json!({
                    "total_pending": total,
                    "matched": matched,
                    "unmatched": total as i64 - matched
                })
                .to_string(),
            )
        },
    );
}
