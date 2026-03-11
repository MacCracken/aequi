use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::ServerState;

async fn health_check(State(state): State<Arc<ServerState>>) -> Json<Value> {
    let account_count = aequi_storage::get_all_accounts(&state.db)
        .await
        .map(|a| a.len())
        .unwrap_or(0);

    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "accounts": account_count,
    }))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/health", get(health_check))
}
