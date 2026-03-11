use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_unpaid_invoices".to_string(),
            description: "List all unpaid invoices".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        false,
        |db, _params| async move {
            let sent = aequi_storage::get_invoices_by_status(&db, "Sent")
                .await
                .unwrap_or_default();
            let viewed = aequi_storage::get_invoices_by_status(&db, "Viewed")
                .await
                .unwrap_or_default();
            let partial = aequi_storage::get_invoices_by_status(&db, "PartiallyPaid")
                .await
                .unwrap_or_default();

            let all: Vec<_> = sent.into_iter().chain(viewed).chain(partial).collect();
            ToolResult::text(serde_json::to_string_pretty(&all).unwrap())
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_draft_invoice".to_string(),
            description: "Create a draft invoice".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "invoice_number": { "type": "string" },
                    "contact_id": { "type": "integer" },
                    "issue_date": { "type": "string" },
                    "due_date": { "type": "string" },
                    "notes": { "type": "string" }
                },
                "required": ["invoice_number", "contact_id", "issue_date", "due_date"]
            }),
        },
        true,
        |db, params| async move {
            let number = params
                .get("invoice_number")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let contact_id = params
                .get("contact_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let issue = params
                .get("issue_date")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let due = params
                .get("due_date")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let notes = params.get("notes").and_then(|v| v.as_str());

            match aequi_storage::insert_invoice(
                &db, number, contact_id, "Draft", None, issue, due, None, None, notes, None,
            )
            .await
            {
                Ok(id) => {
                    ToolResult::text(json!({ "invoice_id": id, "status": "Draft" }).to_string())
                }
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_record_payment".to_string(),
            description: "Record a payment against an invoice".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "invoice_id": { "type": "integer" },
                    "amount_cents": { "type": "integer" },
                    "date": { "type": "string" },
                    "method": { "type": "string" }
                },
                "required": ["invoice_id", "amount_cents", "date"]
            }),
        },
        true,
        |db, params| async move {
            let invoice_id = params
                .get("invoice_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let amount = params
                .get("amount_cents")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let date = params.get("date").and_then(|v| v.as_str()).unwrap_or("");
            let method = params.get("method").and_then(|v| v.as_str());

            match aequi_storage::insert_payment(&db, invoice_id, amount, date, method, None).await {
                Ok(id) => ToolResult::text(json!({ "payment_id": id }).to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
