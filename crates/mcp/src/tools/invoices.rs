use serde_json::{Value, json};

use aequi_storage::{NewDiscount, NewInvoice, NewInvoiceLine, NewInvoiceTaxLine};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, ToolOutcome, not_found};
use crate::tools::{Access, ToolRegistry};

/// Statuses in which an invoice is out with the customer and may be owed.
const OPEN_STATUSES: [&str; 3] = ["Sent", "Viewed", "PartiallyPaid"];

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_list_unpaid_invoices".to_string(),
            description: "List invoices that are sent, viewed or partially paid and still have a balance due, with their totals in cents (total_cents, paid_cents, balance_cents)".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let unpaid: Vec<_> = aequi_storage::list_invoice_summaries(&db)
                .await?
                .into_iter()
                .filter(|s| {
                    s.totals.balance_cents > 0
                        && OPEN_STATUSES.contains(&s.invoice.status_type.as_str())
                })
                .collect();
            Ok(ToolResult::json(&unpaid))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_invoice".to_string(),
            description: "Get an invoice with its line items, tax lines, payments and totals in cents (subtotal, discount, tax, total, paid, balance)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "invoice_id": { "type": "integer" } },
                "required": ["invoice_id"]
            }),
        },
        Access::Read,
        |db, params| async move {
            let invoice_id = Args::new(&params).id("invoice_id")?;
            invoice_detail(&db, invoice_id).await
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_draft_invoice".to_string(),
            description: "Create a draft invoice with line items, optional sales tax lines and an optional discount. Returns the invoice with its computed totals. Use aequi_list_contacts to find the customer's contact_id".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "invoice_number": { "type": "string", "description": "Must be unique" },
                    "contact_id": { "type": "integer", "description": "Customer contact id" },
                    "issue_date": { "type": "string", "description": "YYYY-MM-DD" },
                    "due_date": { "type": "string", "description": "YYYY-MM-DD, not before issue_date" },
                    "lines": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "description": { "type": "string" },
                                "quantity": { "type": "number", "exclusiveMinimum": 0, "description": "Units, e.g. hours; at most 2 decimal places (1.5 = one and a half)" },
                                "unit_rate_cents": { "type": "integer", "minimum": 0, "description": "Price per unit in cents" },
                                "taxable": { "type": "boolean", "default": false, "description": "Whether the tax lines apply to this line" }
                            },
                            "required": ["description", "quantity", "unit_rate_cents"]
                        }
                    },
                    "tax_lines": {
                        "type": "array",
                        "description": "Sales taxes applied to the taxable lines (after the discount)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "e.g. \"State sales tax\"" },
                                "rate_bps": { "type": "integer", "minimum": 0, "maximum": 10000, "description": "Rate in basis points: 825 = 8.25%" }
                            },
                            "required": ["label", "rate_bps"]
                        }
                    },
                    "discount": {
                        "type": "object",
                        "description": "{\"type\": \"percentage\", \"bps\": 1000} for 10% off, or {\"type\": \"flat\", \"cents\": 5000} for $50 off",
                        "properties": {
                            "type": { "type": "string", "enum": ["percentage", "flat"] },
                            "bps": { "type": "integer", "minimum": 0, "maximum": 10000 },
                            "cents": { "type": "integer", "minimum": 0 }
                        },
                        "required": ["type"]
                    },
                    "notes": { "type": "string" },
                    "terms": { "type": "string", "description": "Payment terms, e.g. \"Net 30\"" }
                },
                "required": ["invoice_number", "contact_id", "issue_date", "due_date", "lines"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let invoice = parse_new_invoice(&params)?;
            let contact_id = invoice.contact_id;
            if aequi_storage::get_contact_by_id(&db, contact_id)
                .await?
                .is_none()
            {
                return Err(ToolError(format!("Contact {contact_id} not found")));
            }
            let id = aequi_storage::create_invoice(&db, &invoice).await?;
            invoice_detail(&db, id).await
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_record_payment".to_string(),
            description: "Record a payment received against an invoice. This also posts the cash-basis ledger entry: debit the deposit account (default 1000 Checking; must be an asset account), credit 4000 Services Revenue, and credit 2010 Taxes Payable with the payment's share of the invoice's sales tax. The invoice becomes PartiallyPaid or Paid; a payment may not exceed the balance due".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "invoice_id": { "type": "integer" },
                    "amount_cents": { "type": "integer", "minimum": 1 },
                    "date": { "type": "string", "description": "YYYY-MM-DD" },
                    "method": { "type": "string", "description": "e.g. \"check\", \"ACH\"" },
                    "deposit_account_code": { "type": "string", "description": "Asset account the money went into (default 1000)" }
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
            let deposit = args.opt_str("deposit_account_code")?;

            let payment_id =
                aequi_storage::insert_payment(&db, invoice_id, amount, date, method, deposit)
                    .await
                    .map_err(not_found(format!("Invoice {invoice_id}")))?;
            let detail = aequi_storage::get_invoice_detail(&db, invoice_id)
                .await?
                .ok_or_else(|| ToolError(format!("Invoice {invoice_id} not found")))?;
            let transaction_id = detail
                .payments
                .iter()
                .find(|p| p.id == payment_id)
                .and_then(|p| p.transaction_id);
            Ok(ToolResult::json(&json!({
                "payment_id": payment_id,
                "transaction_id": transaction_id,
                "invoice_id": invoice_id,
                "invoice_status": detail.invoice.status_type,
                "paid_cents": detail.totals.paid_cents,
                "balance_cents": detail.totals.balance_cents
            })))
        },
    );
}

async fn invoice_detail(db: &aequi_storage::DbPool, invoice_id: i64) -> ToolOutcome {
    match aequi_storage::get_invoice_detail(db, invoice_id).await? {
        Some(detail) => Ok(ToolResult::json(&detail)),
        None => Err(ToolError(format!("Invoice {invoice_id} not found"))),
    }
}

fn parse_new_invoice(params: &Value) -> Result<NewInvoice, ToolError> {
    let args = Args::new(params);
    let invoice_number = args.str("invoice_number")?.to_string();
    let contact_id = args.id("contact_id")?;
    let issue_date = args.date("issue_date")?.to_string();
    let due_date = args.date("due_date")?.to_string();

    let lines = args
        .array("lines")?
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = Args::object(line, format!("lines[{i}]"))?;
            line.only(&["description", "quantity", "unit_rate_cents", "taxable"])?;
            Ok(NewInvoiceLine {
                description: line.str("description")?.to_string(),
                quantity_hundredths: line.hundredths("quantity")?,
                unit_rate_cents: line.cents("unit_rate_cents")?,
                taxable: line.opt_bool("taxable")?.unwrap_or(false),
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;

    let tax_lines = args
        .opt_array("tax_lines")?
        .iter()
        .enumerate()
        .map(|(i, tax)| {
            let tax = Args::object(tax, format!("tax_lines[{i}]"))?;
            tax.only(&["label", "rate_bps"])?;
            Ok(NewInvoiceTaxLine {
                label: tax.str("label")?.to_string(),
                rate_bps: tax.i64("rate_bps")?,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;

    let discount = match args.opt_object("discount")? {
        None => None,
        Some(d) => Some(match d.str("type")? {
            "percentage" => {
                d.only(&["type", "bps"])?;
                NewDiscount::Percentage { bps: d.i64("bps")? }
            }
            "flat" => {
                d.only(&["type", "cents"])?;
                NewDiscount::Flat {
                    cents: d.cents("cents")?,
                }
            }
            other => {
                return Err(ToolError(format!(
                    "Argument discount.type must be \"percentage\" or \"flat\", got {other:?}"
                )));
            }
        }),
    };

    Ok(NewInvoice {
        invoice_number,
        contact_id,
        issue_date,
        due_date,
        lines,
        tax_lines,
        discount,
        notes: args.opt_str("notes")?.map(str::to_string),
        terms: args.opt_str("terms")?.map(str::to_string),
    })
}
