use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::ServerState;

fn invoice_not_found(id: i64) -> ApiError {
    ApiError::NotFound(format!("Invoice {id} not found"))
}

/// Storage reports a missing invoice as `RowNotFound`; name it in the error.
fn invoice_error(id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => invoice_not_found(id),
        other => other.into(),
    }
}

async fn invoice_detail(
    state: &ServerState,
    id: i64,
) -> Result<aequi_storage::InvoiceDetail, ApiError> {
    aequi_storage::get_invoice_detail(&state.db, id)
        .await?
        .ok_or_else(|| invoice_not_found(id))
}

/// All invoices with their computed totals and amounts paid.
async fn list_invoices(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_storage::InvoiceSummary>>, ApiError> {
    let invoices = aequi_storage::list_invoice_summaries(&state.db).await?;
    Ok(Json(invoices))
}

/// One invoice with its line items, tax lines, payments and totals.
async fn get_invoice(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
) -> Result<Json<aequi_storage::InvoiceDetail>, ApiError> {
    Ok(Json(invoice_detail(&state, id).await?))
}

/// Create a draft invoice. Storage validates the contents (at least one line
/// item, quantities, rates, dates, contact, unique number) -> 400.
async fn create_invoice(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<aequi_storage::NewInvoice>,
) -> Result<(StatusCode, Json<aequi_storage::InvoiceDetail>), ApiError> {
    let id = aequi_storage::create_invoice(&state.db, &input).await?;
    let detail = aequi_storage::get_invoice_detail(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::Internal("Invoice not found after insert".to_string()))?;
    Ok((StatusCode::CREATED, Json(detail)))
}

/// Replace the contents of a draft invoice. Sent invoices can't be edited;
/// void them and issue a new one instead.
async fn update_invoice(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
    Json(input): Json<aequi_storage::NewInvoice>,
) -> Result<Json<aequi_storage::InvoiceDetail>, ApiError> {
    aequi_storage::update_draft_invoice(&state.db, id, &input)
        .await
        .map_err(invoice_error(id))?;
    Ok(Json(invoice_detail(&state, id).await?))
}

#[derive(Deserialize)]
struct VoidInvoice {
    reason: String,
}

/// Void an unpaid invoice. The reason is kept with the invoice's status.
async fn void_invoice(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
    Json(input): Json<VoidInvoice>,
) -> Result<Json<aequi_storage::InvoiceDetail>, ApiError> {
    let reason = input.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest(
            "A reason is required to void an invoice".to_string(),
        ));
    }
    aequi_storage::void_invoice(&state.db, id, reason)
        .await
        .map_err(invoice_error(id))?;
    Ok(Json(invoice_detail(&state, id).await?))
}

// ── Contacts ─────────────────────────────────────────────────────────────────

async fn list_contacts(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_storage::ContactRecord>>, ApiError> {
    let contacts = aequi_storage::get_all_contacts(&state.db).await?;
    Ok(Json(contacts))
}

#[derive(Deserialize)]
struct CreateContact {
    name: String,
    email: Option<String>,
    phone: Option<String>,
    address: Option<String>,
    contact_type: String,
    is_contractor: bool,
    tax_id: Option<String>,
    notes: Option<String>,
}

async fn create_contact(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateContact>,
) -> Result<Json<aequi_storage::ContactRecord>, ApiError> {
    let id = aequi_storage::insert_contact(
        &state.db,
        &input.name,
        input.email.as_deref(),
        input.phone.as_deref(),
        input.address.as_deref(),
        &input.contact_type,
        input.is_contractor,
        input.tax_id.as_deref(),
        input.notes.as_deref(),
    )
    .await?;

    let record = aequi_storage::get_contact_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::Internal("Contact not found after insert".to_string()))?;
    Ok(Json(record))
}

// ── Payments ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RecordPayment {
    amount_cents: i64,
    date: String,
    method: Option<String>,
    /// Asset account the money was deposited to (default 1000 Checking).
    deposit_account_code: Option<String>,
}

async fn record_payment(
    State(state): State<Arc<ServerState>>,
    Path(invoice_id): Path<i64>,
    Json(input): Json<RecordPayment>,
) -> Result<Json<aequi_storage::PaymentRecord>, ApiError> {
    // Storage validates amount/date/deposit account, rejects Void/Paid
    // invoices and overpayments (-> 400), posts the cash-basis ledger entry,
    // and advances the invoice status, all in one database transaction.
    let id = aequi_storage::insert_payment(
        &state.db,
        invoice_id,
        input.amount_cents,
        &input.date,
        input.method.as_deref(),
        input.deposit_account_code.as_deref(),
    )
    .await
    .map_err(invoice_error(invoice_id))?;

    let payments = aequi_storage::get_payments_for_invoice(&state.db, invoice_id).await?;
    let record = payments
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::Internal("Payment not found after insert".to_string()))?;
    Ok(Json(record))
}

// ── Send invoice via email ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct SendInvoiceRequest {
    subject: Option<String>,
}

fn record_to_contact(rec: &aequi_storage::ContactRecord) -> aequi_core::Contact {
    use aequi_core::*;

    let contact_type = match rec.contact_type.as_str() {
        "Vendor" => ContactType::Vendor,
        "Contractor" => ContactType::Contractor,
        _ => ContactType::Client,
    };

    Contact {
        id: Some(ContactId(rec.id)),
        name: rec.name.clone(),
        email: rec.email.clone(),
        phone: rec.phone.clone(),
        address: rec.address.clone(),
        contact_type,
        is_contractor: rec.is_contractor,
        tax_id: rec.tax_id.clone(),
        notes: rec.notes.clone(),
    }
}

async fn send_invoice(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
    Json(input): Json<SendInvoiceRequest>,
) -> Result<Json<aequi_email::DeliveryResult>, ApiError> {
    let rec = aequi_storage::get_invoice_by_id(&state.db, id)
        .await?
        .ok_or_else(|| invoice_not_found(id))?;
    if rec.status_type == "Void" {
        return Err(ApiError::BadRequest(format!(
            "Invoice {id} is void and cannot be sent"
        )));
    }

    // Invoices created before line items existed total $0; don't bill that.
    let lines = aequi_storage::get_invoice_lines(&state.db, id).await?;
    if lines.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Invoice {id} has no line items; add at least one before sending"
        )));
    }

    let email_config = state
        .email_config
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Email not configured".to_string()))?;

    let tax_lines = aequi_storage::get_invoice_tax_lines(&state.db, id).await?;
    let invoice = aequi_storage::invoice_from_records(&rec, &lines, &tax_lines)?;

    let contact_rec = aequi_storage::get_contact_by_id(&state.db, rec.contact_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Contact not found".to_string()))?;
    let contact = record_to_contact(&contact_rec);

    let result =
        aequi_email::send_invoice(email_config, &invoice, &contact, input.subject.as_deref())
            .await
            .map_err(|e| match e {
                aequi_email::DeliveryError::NoRecipientEmail
                | aequi_email::DeliveryError::MessageBuild(_) => {
                    ApiError::BadRequest(e.to_string())
                }
                other => ApiError::Internal(other.to_string()),
            })?;

    // Draft -> Sent; re-sends leave Sent/Viewed/paid states alone.
    aequi_storage::mark_invoice_sent(&state.db, id, &chrono::Utc::now().to_rfc3339()).await?;

    Ok(Json(result))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/invoices", get(list_invoices).post(create_invoice))
        .route("/invoices/{id}", get(get_invoice).put(update_invoice))
        .route("/invoices/{id}/void", post(void_invoice))
        .route("/invoices/{id}/send", post(send_invoice))
        .route("/invoices/{id}/payments", post(record_payment))
        .route("/contacts", get(list_contacts).post(create_contact))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};
    use serde_json::{Value, json};

    use crate::routes::tests::{call, contact, test_state};
    use crate::state::ServerState;

    /// $150 taxable + $50 non-taxable, 10% sales tax: total $215.00.
    fn new_invoice(number: &str, contact_id: i64) -> Value {
        json!({
            "invoice_number": number,
            "contact_id": contact_id,
            "issue_date": "2026-03-01",
            "due_date": "2026-03-31",
            "lines": [
                {"description": "Design", "quantity_hundredths": 150, "unit_rate_cents": 10_000, "taxable": true},
                {"description": "Hosting", "quantity_hundredths": 100, "unit_rate_cents": 5_000}
            ],
            "tax_lines": [{"label": "Sales tax", "rate_bps": 1000}],
            "notes": "Thanks!"
        })
    }

    async fn create(state: &Arc<ServerState>, body: Value) -> i64 {
        let (status, inv) = call(state, Method::POST, "/api/v1/invoices", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{inv}");
        inv["id"].as_i64().unwrap()
    }

    async fn ledger_lines(state: &Arc<ServerState>, tx_id: i64) -> Vec<(String, i64, i64)> {
        sqlx::query_as(
            "SELECT a.code, tl.debit_cents, tl.credit_cents FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? ORDER BY tl.id",
        )
        .bind(tx_id)
        .fetch_all(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn invoice_with_line_items_has_real_totals() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;

        let (status, inv) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(new_invoice("INV-1", client)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{inv}");
        assert_eq!(inv["status_type"], "Draft");
        assert_eq!(inv["lines"].as_array().unwrap().len(), 2);
        assert_eq!(inv["tax_lines"].as_array().unwrap().len(), 1);
        assert_eq!(inv["payments"], json!([]));
        assert_eq!(inv["subtotal_cents"], 20_000);
        assert_eq!(inv["tax_cents"], 1_500);
        assert_eq!(inv["total_cents"], 21_500);
        assert_eq!(inv["balance_cents"], 21_500);
        let id = inv["id"].as_i64().unwrap();

        let (status, list) = call(&state, Method::GET, "/api/v1/invoices", None).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], id);
        assert_eq!(list[0]["invoice_number"], "INV-1");
        assert_eq!(list[0]["total_cents"], 21_500);
        assert_eq!(list[0]["paid_cents"], 0);

        let (status, detail) =
            call(&state, Method::GET, &format!("/api/v1/invoices/{id}"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["lines"][0]["description"], "Design");
        assert_eq!(detail["lines"][0]["quantity_hundredths"], 150);
        assert_eq!(detail["notes"], "Thanks!");
    }

    #[tokio::test]
    async fn draft_invoice_can_be_edited_until_paid() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;
        let id = create(&state, new_invoice("INV-1", client)).await;
        let uri = format!("/api/v1/invoices/{id}");

        let mut edited = new_invoice("INV-1", client);
        edited["discount"] = json!({"type": "percentage", "bps": 1000});
        let (status, inv) = call(&state, Method::PUT, &uri, Some(edited.clone())).await;
        assert_eq!(status, StatusCode::OK, "{inv}");
        assert_eq!(inv["discount_type"], "Percentage");
        assert_eq!(inv["discount_cents"], 2_000);
        let (sub, disc, tax) = (
            inv["subtotal_cents"].as_i64().unwrap(),
            inv["discount_cents"].as_i64().unwrap(),
            inv["tax_cents"].as_i64().unwrap(),
        );
        assert_eq!(inv["total_cents"].as_i64().unwrap(), sub - disc + tax);

        // An edit that drops every line item is rejected.
        let mut empty = new_invoice("INV-1", client);
        empty["lines"] = json!([]);
        let (status, body) = call(&state, Method::PUT, &uri, Some(empty)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("line item"));

        let (status, _) = call(
            &state,
            Method::POST,
            &format!("{uri}/payments"),
            Some(json!({"amount_cents": 1_000, "date": "2026-03-10"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // No longer a draft.
        let (status, body) = call(&state, Method::PUT, &uri, Some(edited)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("draft"));
    }

    #[tokio::test]
    async fn payment_posts_to_chosen_deposit_account() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;
        let id = create(&state, new_invoice("INV-1", client)).await;
        let payments = format!("/api/v1/invoices/{id}/payments");

        // Half the invoice, deposited to Savings: 7.50 of it is sales tax.
        let (status, payment) = call(
            &state,
            Method::POST,
            &payments,
            Some(json!({
                "amount_cents": 10_750, "date": "2026-03-15",
                "method": "ACH", "deposit_account_code": "1010"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{payment}");
        let tx_id = payment["transaction_id"]
            .as_i64()
            .expect("payment is posted");
        assert_eq!(
            ledger_lines(&state, tx_id).await,
            vec![
                ("1010".to_string(), 10_750, 0),
                ("4000".to_string(), 0, 10_000),
                ("2010".to_string(), 0, 750),
            ]
        );

        // The ledger entry names the client as payer.
        let (_, txs) = call(&state, Method::GET, "/api/v1/transactions", None).await;
        let tx = txs
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == tx_id)
            .unwrap();
        assert_eq!(tx["contact_id"], client);

        let (_, inv) = call(&state, Method::GET, &format!("/api/v1/invoices/{id}"), None).await;
        assert_eq!(inv["status_type"], "PartiallyPaid");
        assert_eq!(inv["paid_cents"], 10_750);
        assert_eq!(inv["balance_cents"], 10_750);
        assert_eq!(inv["payments"].as_array().unwrap().len(), 1);

        // Deposits must go to an asset account.
        let (status, body) = call(
            &state,
            Method::POST,
            &payments,
            Some(
                json!({"amount_cents": 100, "date": "2026-03-16", "deposit_account_code": "4000"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("asset"));

        // Default deposit account is Checking; paying the rest settles it.
        let (status, payment) = call(
            &state,
            Method::POST,
            &payments,
            Some(json!({"amount_cents": 10_750, "date": "2026-03-20"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let lines = ledger_lines(&state, payment["transaction_id"].as_i64().unwrap()).await;
        assert_eq!(lines[0], ("1000".to_string(), 10_750, 0));
        let (_, inv) = call(&state, Method::GET, &format!("/api/v1/invoices/{id}"), None).await;
        assert_eq!(inv["status_type"], "Paid");
        assert_eq!(inv["balance_cents"], 0);
    }

    #[tokio::test]
    async fn invoice_validation_errors_are_400() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;

        let mut no_lines = new_invoice("INV-1", client);
        no_lines["lines"] = json!([]);
        let (status, body) = call(&state, Method::POST, "/api/v1/invoices", Some(no_lines)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("line item"));

        let mut bad_qty = new_invoice("INV-1", client);
        bad_qty["lines"][0]["quantity_hundredths"] = json!(0);
        let (status, _) = call(&state, Method::POST, "/api/v1/invoices", Some(bad_qty)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(new_invoice("INV-1", 999)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        create(&state, new_invoice("INV-1", client)).await;
        let (status, body) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(new_invoice("INV-1", client)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("already in use"));
    }

    #[tokio::test]
    async fn missing_invoice_is_404() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;

        let (status, _) = call(&state, Method::GET, "/api/v1/invoices/999", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = call(
            &state,
            Method::PUT,
            "/api/v1/invoices/999",
            Some(new_invoice("INV-1", client)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Invoice 999 not found");
        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices/999/void",
            Some(json!({"reason": "duplicate"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices/999/send",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn void_invoice_records_reason_and_blocks_payments() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;
        let id = create(&state, new_invoice("INV-1", client)).await;
        let void = format!("/api/v1/invoices/{id}/void");

        let (status, _) = call(&state, Method::POST, &void, Some(json!({"reason": "  "}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, inv) = call(
            &state,
            Method::POST,
            &void,
            Some(json!({"reason": "Billed twice"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{inv}");
        assert_eq!(inv["status_type"], "Void");
        assert!(
            inv["status_data"]
                .as_str()
                .unwrap()
                .contains("Billed twice")
        );

        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/api/v1/invoices/{id}/payments"),
            Some(json!({"amount_cents": 100, "date": "2026-03-10"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // A paid invoice can't be voided.
        let paid = create(&state, new_invoice("INV-2", client)).await;
        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/api/v1/invoices/{paid}/payments"),
            Some(json!({"amount_cents": 21_500, "date": "2026-03-10"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/api/v1/invoices/{paid}/void"),
            Some(json!({"reason": "oops"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn send_refuses_invoice_without_line_items() {
        let state = test_state(None).await;
        let client = contact(&state, "Acme", false).await;

        // Created before invoices had line items: it would bill $0.
        let legacy = aequi_storage::insert_invoice(
            &state.db,
            "OLD-1",
            client,
            "Draft",
            None,
            "2026-01-01",
            "2026-01-31",
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (status, body) = call(
            &state,
            Method::POST,
            &format!("/api/v1/invoices/{legacy}/send"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("no line items"));

        // With line items it gets as far as the (unconfigured) mailer.
        let id = create(&state, new_invoice("INV-1", client)).await;
        let (status, body) = call(
            &state,
            Method::POST,
            &format!("/api/v1/invoices/{id}/send"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "Email not configured");
    }
}
