use std::collections::HashMap;

use serde_json::{Value, json};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, not_found};
use crate::tools::{Access, ToolRegistry};

const DEFAULT_IMPORT_LIMIT: i64 = 100;
const MAX_IMPORT_LIMIT: i64 = 1000;

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
            description: "List imported bank/card transactions awaiting review (status pending or categorized), oldest first, across all import batches unless batch_id is given. amount_cents is negative for money out of the account and positive for money in. suggested_category is the account of the categorization rule that matched, if any. Post each with aequi_post_import or dismiss it with aequi_ignore_import".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "batch_id": { "type": "string", "description": "Only this import batch" },
                    "limit": { "type": "integer", "default": DEFAULT_IMPORT_LIMIT, "minimum": 1, "maximum": MAX_IMPORT_LIMIT }
                }
            }),
        },
        Access::Read,
        |db, params| async move {
            let args = Args::new(&params);
            let batch_id = args.opt_str("batch_id")?;
            let limit = args.opt_i64("limit")?.unwrap_or(DEFAULT_IMPORT_LIMIT);
            if !(1..=MAX_IMPORT_LIMIT).contains(&limit) {
                return Err(ToolError(format!(
                    "Argument limit must be between 1 and {MAX_IMPORT_LIMIT}, got {limit}"
                )));
            }
            let mut rows = match batch_id {
                Some(batch) => {
                    aequi_storage::get_imported_transactions_for_review(&db, batch).await?
                }
                None => aequi_storage::get_pending_imports(&db, limit).await?,
            };
            rows.truncate(limit as usize);

            let rule_accounts: HashMap<i64, (String, String)> =
                sqlx::query_as::<_, (i64, String, String)>(
                    "SELECT r.id, a.code, a.name FROM categorization_rules r \
                     JOIN accounts a ON a.id = r.account_id",
                )
                .fetch_all(&db)
                .await?
                .into_iter()
                .map(|(id, code, name)| (id, (code, name)))
                .collect();

            let out = rows
                .iter()
                .map(|row| {
                    let suggested = row
                        .category_rule_id
                        .and_then(|id| rule_accounts.get(&id))
                        .map(|(code, name)| json!({ "account_code": code, "account_name": name }));
                    let mut v = serde_json::to_value(row).map_err(|e| {
                        ToolError(format!("Failed to serialize imported transaction: {e}"))
                    })?;
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("suggested_category".to_string(), json!(suggested));
                    }
                    Ok(v)
                })
                .collect::<Result<Vec<Value>, ToolError>>()?;
            Ok(ToolResult::json(&out))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_post_import".to_string(),
            description: "Post a reviewed imported transaction to the ledger as a two-line entry between the bank/card account it came from and a category account. Sign convention: a negative amount_cents is money out of the bank/card account (debit the category, credit the bank/card); a positive amount is money in (debit the bank/card, credit the category). Omit category_account_code to use the account of the categorization rule that matched it. Set contact_id to the payee/payer; payments to contractors from a bank account count toward their 1099-NEC".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "imported_id": { "type": "integer", "description": "id from aequi_get_pending_imports" },
                    "bank_account_code": { "type": "string", "description": "The bank (asset) or credit card (liability) account the import came from, e.g. 1000" },
                    "category_account_code": { "type": "string", "description": "Expense, income or other account to post against" },
                    "contact_id": { "type": "integer", "description": "Payee or payer contact id" }
                },
                "required": ["imported_id", "bank_account_code"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let imported_id = args.id("imported_id")?;
            let bank = args.str("bank_account_code")?;
            let category = args.opt_str("category_account_code")?;
            let contact_id = args.opt_id("contact_id")?;
            let tx_id = aequi_storage::post_imported_transaction(
                &db,
                imported_id,
                bank,
                category,
                contact_id,
            )
            .await
            .map_err(not_found(format!("Imported transaction {imported_id}")))?;
            Ok(ToolResult::text(
                json!({ "imported_id": imported_id, "transaction_id": tx_id, "status": "posted" })
                    .to_string(),
            ))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_ignore_import".to_string(),
            description: "Dismiss an imported transaction without posting it, e.g. a transfer or a duplicate already in the ledger".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "imported_id": { "type": "integer", "description": "id from aequi_get_pending_imports" }
                },
                "required": ["imported_id"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let imported_id = Args::new(&params).id("imported_id")?;
            aequi_storage::ignore_imported_transaction(&db, imported_id)
                .await
                .map_err(not_found(format!("Imported transaction {imported_id}")))?;
            Ok(ToolResult::text(
                json!({ "imported_id": imported_id, "status": "ignored" }).to_string(),
            ))
        },
    );
}
