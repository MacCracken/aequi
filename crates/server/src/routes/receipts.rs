use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::error::ApiError;
use crate::state::ServerState;

async fn list_receipts(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_storage::ReceiptRecord>>, ApiError> {
    let receipts = aequi_storage::get_receipts_pending_review(&state.db).await?;
    Ok(Json(receipts))
}

async fn approve_receipt(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
) -> Result<Json<()>, ApiError> {
    aequi_storage::update_receipt_status(&state.db, id, "approved").await?;
    Ok(Json(()))
}

async fn reject_receipt(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
) -> Result<Json<()>, ApiError> {
    aequi_storage::update_receipt_status(&state.db, id, "rejected").await?;
    Ok(Json(()))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/receipts", get(list_receipts))
        .route("/receipts/{id}/approve", post(approve_receipt))
        .route("/receipts/{id}/reject", post(reject_receipt))
}
