mod accounts;
mod health;
mod invoices;
mod receipts;
mod reconciliation;
mod reports;
mod rules;
mod tax;
mod transactions;

use std::sync::Arc;

use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::error::ApiError;
use crate::state::ServerState;

async fn auth_middleware(
    headers: HeaderMap,
    state: axum::extract::State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(ref expected_key) = state.api_key {
        let provided = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match provided {
            Some(key) if key == expected_key => {}
            _ => return Err(ApiError::Unauthorized),
        }
    }
    Ok(next.run(request).await)
}

pub fn router(state: Arc<ServerState>) -> Router {
    let api = Router::new()
        .merge(accounts::routes())
        .merge(transactions::routes())
        .merge(receipts::routes())
        .merge(tax::routes())
        .merge(invoices::routes())
        .merge(rules::routes())
        .merge(reconciliation::routes())
        .merge(reports::routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(health::routes())
        .nest("/api/v1", api)
        .layer(CorsLayer::permissive())
        .with_state(state)
}
