use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::error::ApiError;
use crate::state::ServerState;

async fn list_accounts(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_core::Account>>, ApiError> {
    let accounts = aequi_storage::get_all_accounts(&state.db).await?;
    Ok(Json(accounts))
}

async fn get_account(
    State(state): State<Arc<ServerState>>,
    Path(code): Path<String>,
) -> Result<Json<aequi_core::Account>, ApiError> {
    let account = aequi_storage::get_account_by_code(&state.db, &code)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Account {code} not found")))?;
    Ok(Json(account))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/accounts", get(list_accounts))
        .route("/accounts/{code}", get(get_account))
}
