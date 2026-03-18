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
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::error::ApiError;
use crate::state::ServerState;

/// Constant-time string comparison to prevent timing side-channel attacks.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn auth_middleware(
    headers: HeaderMap,
    state: axum::extract::State<Arc<ServerState>>,
    mut request: Request,
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

    // Try API key first (constant-time comparison)
    if let Some(ref expected_key) = state.api_key {
        if constant_time_eq(token, expected_key) {
            return Ok(next.run(request).await);
        }
    }

    // Try OIDC JWT validation — only if token looks like a JWT (has 2 dots)
    if let Some(ref oidc) = state.oidc {
        if token.matches('.').count() == 2 {
            if let Ok(claims) = oidc.validate_token(token).await {
                // Store claims in request extensions for downstream handlers
                request.extensions_mut().insert(claims);
                return Ok(next.run(request).await);
            }
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

    // Restrict CORS to configured origins or localhost for development
    let cors = match std::env::var("AEQUI_CORS_ORIGINS") {
        Ok(origins) => {
            let allowed: Vec<_> = origins
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            CorsLayer::new().allow_origin(AllowOrigin::list(allowed))
        }
        Err(_) => {
            // Default: allow localhost origins for development
            let dev_origins: Vec<_> = [
                "http://localhost:1420",
                "http://localhost:8060",
                "tauri://localhost",
            ]
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
            CorsLayer::new().allow_origin(AllowOrigin::list(dev_origins))
        }
    };

    Router::new()
        .merge(health::routes())
        .nest("/api/v1", api)
        .nest("/api/v1", stripe_api)
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10MB max body
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matching() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("", "a"));
    }
}
