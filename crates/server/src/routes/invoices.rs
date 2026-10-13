use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
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
    if aequi_storage::get_contact_by_id(&state.db, input.contact_id)
        .await?
        .is_none()
    {
        return Err(ApiError::BadRequest(format!(
            "Contact {} not found",
            input.contact_id
        )));
    }

    // Storage validates the dates (YYYY-MM-DD, due >= issue) -> 400.
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
    // Storage validates amount/date, rejects Void/Paid invoices and
    // overpayments (-> 400), and advances the invoice status itself.
    let id = aequi_storage::insert_payment(
        &state.db,
        invoice_id,
        input.amount_cents,
        &input.date,
        input.method.as_deref(),
        None,
    )
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("Invoice {invoice_id} not found")),
        other => other.into(),
    })?;

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
    let email_config = state
        .email_config
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Email not configured".to_string()))?;

    let rec = aequi_storage::get_invoice_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Invoice {id} not found")))?;
    if rec.status_type == "Void" {
        return Err(ApiError::BadRequest(format!(
            "Invoice {id} is void and cannot be sent"
        )));
    }

    let lines = aequi_storage::get_invoice_lines(&state.db, id).await?;
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
        .route("/invoices/{id}", get(get_invoice))
        .route("/invoices/{id}/send", post(send_invoice))
        .route("/invoices/{id}/payments", post(record_payment))
        .route("/contacts", get(list_contacts).post(create_contact))
}
