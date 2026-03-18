use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::ServerState;

async fn list_invoices(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_storage::InvoiceRecord>>, ApiError> {
    let invoices = aequi_storage::get_all_invoices(&state.db).await?;
    Ok(Json(invoices))
}

async fn get_invoice(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
) -> Result<Json<aequi_storage::InvoiceRecord>, ApiError> {
    let invoice = aequi_storage::get_invoice_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Invoice {id} not found")))?;
    Ok(Json(invoice))
}

#[derive(Deserialize)]
struct CreateInvoice {
    invoice_number: String,
    contact_id: i64,
    issue_date: String,
    due_date: String,
    notes: Option<String>,
    terms: Option<String>,
}

async fn create_invoice(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateInvoice>,
) -> Result<Json<aequi_storage::InvoiceRecord>, ApiError> {
    let id = aequi_storage::insert_invoice(
        &state.db,
        &input.invoice_number,
        input.contact_id,
        "Draft",
        None,
        &input.issue_date,
        &input.due_date,
        None,
        None,
        input.notes.as_deref(),
        input.terms.as_deref(),
    )
    .await?;

    let record = aequi_storage::get_invoice_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::Internal("Invoice not found after insert".to_string()))?;
    Ok(Json(record))
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
}

async fn record_payment(
    State(state): State<Arc<ServerState>>,
    Path(invoice_id): Path<i64>,
    Json(input): Json<RecordPayment>,
) -> Result<Json<aequi_storage::PaymentRecord>, ApiError> {
    let id = aequi_storage::insert_payment(
        &state.db,
        invoice_id,
        input.amount_cents,
        &input.date,
        input.method.as_deref(),
        None,
    )
    .await?;

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

/// Reconstruct a domain Invoice from storage records.
fn records_to_invoice(
    rec: &aequi_storage::InvoiceRecord,
    lines: &[aequi_storage::InvoiceLineRecord],
    tax_lines: &[aequi_storage::InvoiceTaxLineRecord],
) -> aequi_core::Invoice {
    use aequi_core::*;
    use chrono::NaiveDate;

    let discount = match (rec.discount_type.as_deref(), rec.discount_value) {
        (Some("Percentage"), Some(bps)) => Some(Discount::Percentage(Decimal::new(bps, 2))),
        (Some("Flat"), Some(cents)) => Some(Discount::Flat(Money::from_cents(cents))),
        _ => None,
    };

    Invoice {
        id: Some(InvoiceId(rec.id)),
        invoice_number: rec.invoice_number.clone(),
        contact_id: ContactId(rec.contact_id),
        status: InvoiceStatus::Draft, // status not needed for rendering
        issue_date: NaiveDate::parse_from_str(&rec.issue_date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Utc::now().date_naive()),
        due_date: NaiveDate::parse_from_str(&rec.due_date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Utc::now().date_naive()),
        lines: lines
            .iter()
            .map(|l| InvoiceLine {
                description: l.description.clone(),
                quantity: Decimal::new(l.quantity_hundredths, 2),
                unit_rate: Money::from_cents(l.unit_rate_cents),
                taxable: l.taxable,
            })
            .collect(),
        discount,
        tax_lines: tax_lines
            .iter()
            .map(|t| TaxLine {
                label: t.label.clone(),
                rate: Decimal::new(t.rate_bps, 4),
            })
            .collect(),
        notes: rec.notes.clone(),
        terms: rec.terms.clone(),
    }
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
    let email_config = state
        .email_config
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Email not configured".to_string()))?;

    let rec = aequi_storage::get_invoice_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Invoice {id} not found")))?;

    let lines = aequi_storage::get_invoice_lines(&state.db, id).await?;
    let tax_lines = aequi_storage::get_invoice_tax_lines(&state.db, id).await?;
    let invoice = records_to_invoice(&rec, &lines, &tax_lines);

    let contact_rec = aequi_storage::get_contact_by_id(&state.db, rec.contact_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Contact not found".to_string()))?;
    let contact = record_to_contact(&contact_rec);

    let result =
        aequi_email::send_invoice(email_config, &invoice, &contact, input.subject.as_deref())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Update invoice status to Sent
    let sent_data = serde_json::json!({ "sent_at": chrono::Utc::now().to_rfc3339() });
    let _ =
        aequi_storage::update_invoice_status(&state.db, id, "Sent", Some(&sent_data.to_string()))
            .await;

    Ok(Json(result))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/invoices", get(list_invoices).post(create_invoice))
        .route("/invoices/{id}", get(get_invoice))
        .route("/invoices/{id}/send", post(send_invoice))
        .route("/invoices/{id}/payments", post(record_payment))
        .route("/contacts", get(list_contacts).post(create_contact))
}
