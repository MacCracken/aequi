mod accounts;
mod health;
mod invoices;
mod plaid;
mod receipts;
mod reconciliation;
mod reports;
mod rules;
mod stripe;
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
    // If neither API key nor OIDC is configured, allow all requests
    if state.api_key.is_none() && state.oidc.is_none() {
        return Ok(next.run(request).await);
    }

    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t,
        None => return Err(ApiError::Unauthorized),
    };

    // Try API key first
    if let Some(ref expected_key) = state.api_key {
        if token == expected_key {
            return Ok(next.run(request).await);
        }
    }

    // Try OIDC JWT validation
    if let Some(ref oidc) = state.oidc {
        if oidc.validate_token(token).await.is_ok() {
            return Ok(next.run(request).await);
        }
    }

    Err(ApiError::Unauthorized)
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
        .merge(plaid::routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Stripe webhook sits outside auth middleware (has its own signature verification)
    let stripe_api = Router::new()
        .merge(stripe::routes());

    Router::new()
        .merge(health::routes())
        .nest("/api/v1", api)
        .nest("/api/v1", stripe_api)
        .layer(CorsLayer::permissive())
        .with_state(state)
}
