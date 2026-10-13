mod accounts;
mod health;
mod imports;
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

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::error::ApiError;
use crate::state::ServerState;

/// Constant-time comparison of secrets. Both sides are hashed first so the
/// comparison time doesn't reveal the secret's length either.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    let (ha, hb) = (Sha256::digest(a), Sha256::digest(b));
    ha.iter()
        .zip(hb.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn auth_middleware(
    headers: HeaderMap,
    state: axum::extract::State<Arc<ServerState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Neither API key nor OIDC configured: startup only allows this on a
    // loopback address or with AEQUI_INSECURE_NO_AUTH=1 (see main.rs).
    if state.api_key.is_none() && state.oidc.is_none() {
        return Ok(next.run(request).await);
    }

    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());

    let token = match token {
        Some(t) => t,
        None => return Err(ApiError::Unauthorized),
    };

    if let Some(ref expected_key) = state.api_key
        && constant_time_eq(token.as_bytes(), expected_key.as_bytes())
    {
        return Ok(next.run(request).await);
    }

    // Try OIDC JWT validation — only if token looks like a JWT (has 2 dots)
    if let Some(ref oidc) = state.oidc
        && token.matches('.').count() == 2
    {
        match oidc.validate_token(token).await {
            Ok(claims) => {
                // Store claims in request extensions for downstream handlers
                request.extensions_mut().insert(claims);
                return Ok(next.run(request).await);
            }
            Err(e) => tracing::debug!(error = %e, "OIDC token rejected"),
        }
    }

    Err(ApiError::Unauthorized)
}

/// Build the CORS layer from `AEQUI_CORS_ORIGINS` (comma-separated origins,
/// or `*` for any origin). Unset means the local development origins.
/// Credentials are never allowed: the API authenticates with bearer tokens.
pub fn cors_layer(origins: Option<&str>) -> Result<CorsLayer, String> {
    let allow_origin = match origins.map(str::trim) {
        Some(list) if list.split(',').any(|o| o.trim() == "*") => AllowOrigin::any(),
        Some(list) => {
            let mut allowed = Vec::new();
            for origin in list.split(',').map(str::trim).filter(|o| !o.is_empty()) {
                let value = HeaderValue::from_str(origin)
                    .map_err(|_| format!("invalid origin in AEQUI_CORS_ORIGINS: {origin:?}"))?;
                allowed.push(value);
            }
            AllowOrigin::list(allowed)
        }
        None => AllowOrigin::list([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://localhost:8060"),
            HeaderValue::from_static("tauri://localhost"),
        ]),
    };

    Ok(CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]))
}

pub fn router(state: Arc<ServerState>, cors: CorsLayer) -> Router {
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
        .merge(imports::routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Stripe webhook sits outside auth middleware (has its own signature verification)
    let stripe_api = Router::new().merge(stripe::routes());

    Router::new()
        .merge(health::routes())
        .nest("/api/v1", api)
        .nest("/api/v1", stripe_api)
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10MB max body
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    pub(crate) async fn test_state(api_key: Option<&str>) -> Arc<ServerState> {
        let db = aequi_storage::create_db(&std::path::PathBuf::from(":memory:"))
            .await
            .unwrap();
        aequi_storage::seed_default_accounts(&db).await.unwrap();
        Arc::new(ServerState {
            db,
            api_key: api_key.map(Into::into),
            email_config: None,
            oidc: None,
            stripe_webhook_secret: None,
            stripe_api: None,
            plaid: None,
            plaid_sync_lock: Default::default(),
        })
    }

    /// Create a contact directly in storage; returns its id.
    pub(crate) async fn contact(state: &ServerState, name: &str, is_contractor: bool) -> i64 {
        let contact_type = if is_contractor {
            "Contractor"
        } else {
            "Client"
        };
        aequi_storage::insert_contact(
            &state.db,
            name,
            Some("billing@example.com"),
            None,
            None,
            contact_type,
            is_contractor,
            None,
            None,
        )
        .await
        .unwrap()
    }

    async fn get_status(app: Router, auth: Option<&str>) -> StatusCode {
        let mut req = Request::builder().uri("/api/v1/accounts");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        app.oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[test]
    fn constant_time_eq_matching() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[tokio::test]
    async fn api_key_required_when_configured() {
        let state = test_state(Some("s3cret-key")).await;
        let app = || router(state.clone(), cors_layer(None).unwrap());
        assert_eq!(get_status(app(), None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            get_status(app(), Some("Bearer wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get_status(app(), Some("Bearer ")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get_status(app(), Some("Bearer s3cret-key")).await,
            StatusCode::OK
        );
    }

    #[test]
    fn cors_wildcard_does_not_panic() {
        // tower-http panics on "*" inside AllowOrigin::list.
        assert!(cors_layer(Some("*")).is_ok());
        assert!(cors_layer(Some("http://a.example, *")).is_ok());
        assert!(cors_layer(Some("http://a.example,http://b.example")).is_ok());
        assert!(cors_layer(Some("http://bad\norigin")).is_err());
    }

    #[tokio::test]
    async fn cors_preflight_allows_auth_header() {
        let state = test_state(Some("k")).await;
        let app = router(state, cors_layer(Some("http://app.example")).unwrap());
        let req = Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/v1/transactions")
            .header("origin", "http://app.example")
            .header("access-control-request-method", "POST")
            .header(
                "access-control-request-headers",
                "authorization,content-type",
            )
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().is_success());
        let h = resp.headers();
        assert_eq!(
            h.get("access-control-allow-origin").unwrap(),
            "http://app.example"
        );
        let allowed = h
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap()
            .to_ascii_lowercase();
        assert!(allowed.contains("authorization") && allowed.contains("content-type"));
        assert!(
            h.get("access-control-allow-methods")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("POST")
        );
    }

    /// Serve `app` on an ephemeral loopback port (a stand-in for an external
    /// API); returns its base URL.
    pub(crate) async fn mock_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    /// Send a request to a fresh unauthenticated router; returns status and JSON body.
    pub(crate) async fn call(
        state: &Arc<ServerState>,
        method: Method,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let app = router(state.clone(), cors_layer(None).unwrap());
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                req = req.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn validation_errors_are_400_and_missing_rows_404() {
        let state = test_state(None).await;

        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/tax/payments",
            Some(serde_json::json!({"year": 2026, "quarter": 5, "amount_cents": 100, "date": "2026-04-15"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = call(
            &state,
            Method::POST,
            "/api/v1/tax/payments",
            Some(serde_json::json!({"year": 2026, "quarter": 1, "amount_cents": 100, "date": "04/15/2026"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("YYYY-MM-DD"));

        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/tax/payments",
            Some(serde_json::json!({"year": 2026, "quarter": 1, "amount_cents": 100, "date": "2026-04-15"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = call(
            &state,
            Method::GET,
            "/api/v1/reports/profit-loss?start_date=2026-13-01",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = call(&state, Method::POST, "/api/v1/receipts/999/approve", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices/999/payments",
            Some(serde_json::json!({"amount_cents": 100, "date": "2026-04-15"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/transactions",
            Some(serde_json::json!({
                "date": "2026-04-15",
                "description": "x",
                "lines": [
                    {"account_code": "1000", "debit_cents": 100, "credit_cents": 100},
                    {"account_code": "4000", "debit_cents": 0, "credit_cents": 0}
                ]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invoice_dates_are_validated() {
        let state = test_state(None).await;
        let (status, contact) = call(
            &state,
            Method::POST,
            "/api/v1/contacts",
            Some(serde_json::json!({
                "name": "Acme, Inc.", "contact_type": "Client", "is_contractor": false
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let contact_id = contact["id"].as_i64().unwrap();

        let invoice = |issue: &str, due: &str| {
            serde_json::json!({
                "invoice_number": format!("INV-{issue}-{due}"),
                "contact_id": contact_id,
                "issue_date": issue,
                "due_date": due,
                "lines": [{"description": "Consulting", "quantity_hundredths": 100, "unit_rate_cents": 10_000}],
            })
        };
        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(invoice("2026-03-01", "2026-02-01")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(invoice("3/1/2026", "2026-04-01")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(
            &state,
            Method::POST,
            "/api/v1/invoices",
            Some(invoice("2026-03-01", "2026-03-31")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn quarterly_estimate_reports_annualized_fields() {
        let state = test_state(None).await;
        let (status, body) = call(
            &state,
            Method::GET,
            "/api/v1/tax/quarterly-estimate?year=2026&quarter=2",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for field in [
            "annualization_factor",
            "annualized_net_profit_cents",
            "additional_medicare_tax_cents",
            "standard_deduction_cents",
            "qbi_deduction_cents",
            "taxable_income_cents",
            "safe_harbor_prior_year_percentage",
            "through",
        ] {
            assert!(body.get(field).is_some(), "missing {field} in {body}");
        }

        let (status, _) = call(
            &state,
            Method::GET,
            "/api/v1/tax/quarterly-estimate?year=2026&quarter=9",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = call(
            &state,
            Method::GET,
            "/api/v1/tax/schedule-c?year=2026",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
