use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::ServerState;

#[derive(Deserialize)]
struct CreateSession {
    account_id: i64,
    start_date: String,
    end_date: String,
    statement_balance_cents: i64,
}

async fn create_session(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateSession>,
) -> Result<Json<i64>, ApiError> {
    let id = aequi_storage::create_reconciliation_session(
        &state.db,
        input.account_id,
        &input.start_date,
        &input.end_date,
        input.statement_balance_cents,
    )
    .await?;
    Ok(Json(id))
}

async fn get_items(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<i64>,
) -> Result<Json<Vec<aequi_storage::ReconciliationItem>>, ApiError> {
    let items = aequi_storage::get_reconciliation_items(&state.db, session_id).await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
struct ResolveInput {
    notes: String,
}

async fn resolve_item(
    State(state): State<Arc<ServerState>>,
    Path(item_id): Path<i64>,
    Json(input): Json<ResolveInput>,
) -> Result<Json<()>, ApiError> {
    aequi_storage::resolve_reconciliation_item(&state.db, item_id, &input.notes).await?;
    Ok(Json(()))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/reconciliation/sessions", post(create_session))
        .route("/reconciliation/sessions/{id}/items", get(get_items))
        .route("/reconciliation/items/{id}/resolve", post(resolve_item))
}
