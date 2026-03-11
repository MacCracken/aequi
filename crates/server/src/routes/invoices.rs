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

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/invoices", get(list_invoices).post(create_invoice))
        .route("/invoices/{id}", get(get_invoice))
        .route("/invoices/{id}/payments", post(record_payment))
        .route("/contacts", get(list_contacts).post(create_contact))
}
