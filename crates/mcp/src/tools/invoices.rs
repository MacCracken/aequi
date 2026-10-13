use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, not_found};
use crate::tools::{Access, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_unpaid_invoices".to_string(),
            description: "List all unpaid invoices".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let mut all = Vec::new();
            for status in ["Sent", "Viewed", "PartiallyPaid"] {
                all.extend(aequi_storage::get_invoices_by_status(&db, status).await?);
            }
            Ok(ToolResult::json(&all))
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
                    "issue_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "due_date": { "type": "string", "description": "YYYY-MM-DD, not before issue_date" },
                    "notes": { "type": "string" }
                },
                "required": ["invoice_number", "contact_id", "issue_date", "due_date"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let number = args.str("invoice_number")?;
            let contact_id = args.id("contact_id")?;
            let issue = args.date("issue_date")?;
            let due = args.date("due_date")?;
            let notes = args.opt_str("notes")?;

            if aequi_storage::get_contact_by_id(&db, contact_id)
                .await?
                .is_none()
            {
                return Err(ToolError(format!("Contact {contact_id} not found")));
            }

            let id = aequi_storage::insert_invoice(
                &db, number, contact_id, "Draft", None, issue, due, None, None, notes, None,
            )
            .await?;
            Ok(ToolResult::text(
                json!({ "invoice_id": id, "status": "Draft" }).to_string(),
            ))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_record_payment".to_string(),
            description:
                "Record a payment against an invoice; the invoice becomes PartiallyPaid or Paid"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "invoice_id": { "type": "integer" },
                    "amount_cents": { "type": "integer", "minimum": 1 },
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "method": { "type": "string" }
                },
                "required": ["invoice_id", "amount_cents", "date"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let invoice_id = args.id("invoice_id")?;
            let amount = args.cents("amount_cents")?;
            let date = args.date("date")?;
            let method = args.opt_str("method")?;

            let id = aequi_storage::insert_payment(&db, invoice_id, amount, date, method, None)
                .await
                .map_err(not_found(format!("Invoice {invoice_id}")))?;
            Ok(ToolResult::text(json!({ "payment_id": id }).to_string()))
        },
    );
}
