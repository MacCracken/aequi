use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::ServerState;

async fn list_rules(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<aequi_storage::CategorizationRule>>, ApiError> {
    let rules = aequi_storage::get_categorization_rules(&state.db).await?;
    Ok(Json(rules))
}

#[derive(Deserialize)]
struct CreateRule {
    name: String,
    priority: i32,
    match_pattern: String,
    match_type: String,
    account_id: i64,
}

async fn create_rule(
    State(state): State<Arc<ServerState>>,
    Json(input): Json<CreateRule>,
) -> Result<Json<i64>, ApiError> {
    let rule = aequi_storage::CategorizationRule {
        id: 0,
        name: input.name,
        priority: input.priority,
        match_pattern: input.match_pattern,
        match_type: input.match_type,
        account_id: input.account_id,
        created_at: String::new(),
    };
    let id = aequi_storage::save_categorization_rule(&state.db, &rule).await?;
    Ok(Json(id))
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/rules", get(list_rules).post(create_rule))
}
