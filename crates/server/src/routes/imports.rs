//! Review queue for imported bank transactions (CSV/OFX imports and Plaid
//! syncs). Nothing an import produces reaches the ledger until it is posted
//! here.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::ServerState;

const DEFAULT_PENDING_LIMIT: i64 = 100;
const MAX_PENDING_LIMIT: i64 = 1000;

#[derive(Deserialize)]
struct PendingQuery {
    limit: Option<i64>,
}

/// Storage reports a missing import as `RowNotFound`; name it in the error.
fn import_error(id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("Import {id} not found")),
        other => other.into(),
    }
}

/// GET /imports/pending — imports awaiting review (pending or categorized),
/// oldest first, across all batches.
async fn list_pending(
    State(state): State<Arc<ServerState>>,
    Query(q): Query<PendingQuery>,
) -> Result<Json<Vec<aequi_storage::ImportedTransaction>>, ApiError> {
    let limit = q.limit.unwrap_or(DEFAULT_PENDING_LIMIT);
    if !(1..=MAX_PENDING_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit must be between 1 and {MAX_PENDING_LIMIT}"
        )));
    }
    Ok(Json(
        aequi_storage::get_pending_imports(&state.db, limit).await?,
    ))
}

#[derive(Deserialize)]
struct PostImport {
    /// The bank (asset) or card (liability) account the import came from.
    bank_account_code: String,
    /// Defaults to the account of the categorization rule the import matched.
    category_account_code: Option<String>,
    /// Payee or payer; payments to contractors count toward 1099-NEC.
    contact_id: Option<i64>,
}

#[derive(Serialize)]
struct PostedImport {
    transaction_id: i64,
}

/// POST /imports/{id}/post — post a reviewed import to the ledger.
async fn post_import(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
    Json(input): Json<PostImport>,
) -> Result<(StatusCode, Json<PostedImport>), ApiError> {
    let transaction_id = aequi_storage::post_imported_transaction(
        &state.db,
        id,
        &input.bank_account_code,
        input.category_account_code.as_deref(),
        input.contact_id,
    )
    .await
    .map_err(import_error(id))?;
    Ok((StatusCode::CREATED, Json(PostedImport { transaction_id })))
}

/// POST /imports/{id}/ignore — dismiss an import (e.g. a transfer already
/// recorded another way).
async fn ignore_import(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    aequi_storage::ignore_imported_transaction(&state.db, id)
        .await
        .map_err(import_error(id))?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/imports/pending", get(list_pending))
        .route("/imports/{id}/post", post(post_import))
        .route("/imports/{id}/ignore", post(ignore_import))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use crate::routes::tests::{call, contact, test_state};
    use crate::state::ServerState;

    async fn queue(state: &ServerState, source_id: &str, date: &str, amount_cents: i64) -> i64 {
        aequi_storage::insert_imported_transaction(
            &state.db,
            &aequi_storage::ImportedTransaction {
                id: 0,
                source_type: "plaid".to_string(),
                source_id: Some(source_id.to_string()),
                import_batch_id: "plaid-test".to_string(),
                date: date.to_string(),
                description: format!("Bank item {source_id}"),
                amount_cents,
                debit_cents: (amount_cents < 0).then_some(-amount_cents),
                credit_cents: (amount_cents > 0).then_some(amount_cents),
                memo: Some("Plaid account acc_1 (Business Checking)".to_string()),
                matched_transaction_id: None,
                category_rule_id: None,
                status: "pending".to_string(),
                created_at: String::new(),
            },
        )
        .await
        .unwrap()
    }

    async fn ledger_lines(state: &Arc<ServerState>, tx_id: i64) -> Vec<(String, i64, i64)> {
        sqlx::query_as(
            "SELECT a.code, tl.debit_cents, tl.credit_cents FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? ORDER BY tl.id",
        )
        .bind(tx_id)
        .fetch_all(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn pending_imports_are_listed_oldest_first() {
        let state = test_state(None).await;
        queue(&state, "b", "2026-03-05", -2_000).await;
        queue(&state, "a", "2026-03-01", 10_000).await;
        queue(&state, "c", "2026-03-09", -500).await;

        let (status, list) = call(&state, Method::GET, "/api/v1/imports/pending", None).await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<_> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["source_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(list[0]["amount_cents"], 10_000);
        assert_eq!(list[0]["status"], "pending");

        let (status, list) =
            call(&state, Method::GET, "/api/v1/imports/pending?limit=2", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list.as_array().unwrap().len(), 2);

        for bad in ["0", "1001", "-5"] {
            let (status, _) = call(
                &state,
                Method::GET,
                &format!("/api/v1/imports/pending?limit={bad}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "limit={bad}");
        }
    }

    #[tokio::test]
    async fn posting_an_import_creates_a_ledger_transaction() {
        let state = test_state(None).await;
        let designer = contact(&state, "Pat Designer", true).await;
        let out = queue(&state, "out", "2026-03-02", -42_500).await;
        let deposit = queue(&state, "in", "2026-03-03", 100_000).await;

        // Money out: debit the category, credit the bank, payee recorded.
        let (status, body) = call(
            &state,
            Method::POST,
            &format!("/api/v1/imports/{out}/post"),
            Some(json!({
                "bank_account_code": "1000",
                "category_account_code": "5080",
                "contact_id": designer
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let tx_id = body["transaction_id"].as_i64().unwrap();
        assert_eq!(
            ledger_lines(&state, tx_id).await,
            vec![
                ("5080".to_string(), 42_500, 0),
                ("1000".to_string(), 0, 42_500)
            ]
        );
        let (contact_id,): (Option<i64>,) =
            sqlx::query_as("SELECT contact_id FROM transactions WHERE id = ?")
                .bind(tx_id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(contact_id, Some(designer));

        // Money in: debit the bank, credit the category.
        let (status, body) = call(
            &state,
            Method::POST,
            &format!("/api/v1/imports/{deposit}/post"),
            Some(json!({"bank_account_code": "1000", "category_account_code": "4000"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            ledger_lines(&state, body["transaction_id"].as_i64().unwrap()).await,
            vec![
                ("1000".to_string(), 100_000, 0),
                ("4000".to_string(), 0, 100_000)
            ]
        );

        // Posted imports leave the queue and can't be posted again.
        let (_, list) = call(&state, Method::GET, "/api/v1/imports/pending", None).await;
        assert_eq!(list, json!([]));
        let (status, body) = call(
            &state,
            Method::POST,
            &format!("/api/v1/imports/{out}/post"),
            Some(json!({"bank_account_code": "1000", "category_account_code": "5080"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("already posted"));
    }

    #[tokio::test]
    async fn posting_an_import_validates_its_accounts() {
        let state = test_state(None).await;
        let id = queue(&state, "x", "2026-03-02", -1_000).await;
        let uri = format!("/api/v1/imports/{id}/post");

        for body in [
            // No category given and no categorization rule matched.
            json!({"bank_account_code": "1000"}),
            // The "bank" must be an asset or liability account.
            json!({"bank_account_code": "5080", "category_account_code": "5000"}),
            json!({"bank_account_code": "9999", "category_account_code": "5000"}),
            json!({"bank_account_code": "1000", "category_account_code": "1000"}),
            json!({"bank_account_code": "1000", "category_account_code": "5000", "contact_id": 999}),
        ] {
            let (status, resp) = call(&state, Method::POST, &uri, Some(body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {resp}");
        }

        // Still pending: nothing was posted.
        let (_, list) = call(&state, Method::GET, "/api/v1/imports/pending", None).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ignoring_an_import_removes_it_from_the_queue() {
        let state = test_state(None).await;
        let id = queue(&state, "x", "2026-03-02", -1_000).await;
        let uri = format!("/api/v1/imports/{id}/ignore");

        let (status, _) = call(&state, Method::POST, &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, list) = call(&state, Method::GET, "/api/v1/imports/pending", None).await;
        assert_eq!(list, json!([]));

        let (status, body) = call(&state, Method::POST, &uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("already ignored"));

        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/api/v1/imports/{id}/post"),
            Some(json!({"bank_account_code": "1000", "category_account_code": "5000"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_import_is_404() {
        let state = test_state(None).await;
        let (status, body) = call(
            &state,
            Method::POST,
            "/api/v1/imports/999/post",
            Some(json!({"bank_account_code": "1000", "category_account_code": "5000"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "Import 999 not found");
        let (status, _) = call(&state, Method::POST, "/api/v1/imports/999/ignore", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn imports_require_auth_when_configured() {
        let state = test_state(Some("k")).await;
        let (status, _) = call(&state, Method::GET, "/api/v1/imports/pending", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
