use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};
use aequi_import::plaid::PlaidClient;

use crate::error::ApiError;
use crate::state::ServerState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateLinkTokenRequest {
    /// A unique identifier for the user (e.g. "default" for single-user setups).
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateLinkTokenResponse {
    pub link_token: String,
    pub expiration: String,
}

#[derive(Debug, Deserialize)]
pub struct ExchangePublicTokenRequest {
    pub public_token: String,
}

#[derive(Debug, Serialize)]
pub struct ExchangeResponse {
    pub item_id: String,
    pub stored: bool,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    /// Start date in YYYY-MM-DD format.
    pub start_date: String,
    /// End date in YYYY-MM-DD format.
    pub end_date: String,
}

#[derive(Debug, Serialize)]
pub struct SyncResponse {
    pub transactions_imported: usize,
    pub transactions_skipped: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn plaid_client(state: &ServerState) -> Result<PlaidClient, ApiError> {
    let config = state
        .plaid_config
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Plaid is not configured".to_string()))?;
    Ok(PlaidClient::new(config.clone()))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /plaid/link-token — create a Plaid Link token for the frontend.
async fn create_link_token(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<CreateLinkTokenRequest>,
) -> Result<Json<CreateLinkTokenResponse>, ApiError> {
    let client = plaid_client(&state)?;
    let user_id = body.user_id.as_deref().unwrap_or("default");

    let resp = client
        .create_link_token(user_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Plaid link token error: {e}")))?;

    Ok(Json(CreateLinkTokenResponse {
        link_token: resp.link_token,
        expiration: resp.expiration,
    }))
}

/// POST /plaid/exchange — exchange a public token for an access token and store it.
async fn exchange_public_token(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<ExchangePublicTokenRequest>,
) -> Result<Json<ExchangeResponse>, ApiError> {
    let client = plaid_client(&state)?;

    let resp = client
        .exchange_public_token(&body.public_token)
        .await
        .map_err(|e| ApiError::Internal(format!("Plaid exchange error: {e}")))?;

    // Store the access token in the settings table for later use.
    aequi_storage::set_setting(&state.db, "plaid_access_token", &resp.access_token)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to store access token: {e}")))?;
    aequi_storage::set_setting(&state.db, "plaid_item_id", &resp.item_id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to store item id: {e}")))?;

    Ok(Json(ExchangeResponse {
        item_id: resp.item_id,
        stored: true,
    }))
}

/// POST /plaid/sync — fetch recent transactions from Plaid and import them.
async fn sync_transactions(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<SyncRequest>,
) -> Result<Json<SyncResponse>, ApiError> {
    let client = plaid_client(&state)?;

    let access_token = aequi_storage::get_setting(&state.db, "plaid_access_token")
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read access token: {e}")))?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "No Plaid access token stored. Complete the Link flow first.".to_string(),
            )
        })?;

    let resp = client
        .get_transactions(&access_token, &body.start_date, &body.end_date)
        .await
        .map_err(|e| ApiError::Internal(format!("Plaid transactions error: {e}")))?;

    // Look up accounts for Plaid imports
    let checking = aequi_storage::get_account_by_code(&state.db, "1000")
        .await?
        .ok_or_else(|| ApiError::Internal("Checking account (1000) not found".to_string()))?;

    let expense = aequi_storage::get_account_by_code(&state.db, "5000")
        .await?
        .ok_or_else(|| ApiError::Internal("Expense account (5000) not found".to_string()))?;

    // Use revenue account for income (negative Plaid amounts = inflows)
    let revenue = aequi_storage::get_account_by_code(&state.db, "4000")
        .await?
        .ok_or_else(|| ApiError::Internal("Revenue account (4000) not found".to_string()))?;

    let checking_id = checking.id.unwrap_or(aequi_core::AccountId(0));
    let expense_id = expense.id.unwrap_or(aequi_core::AccountId(0));
    let revenue_id = revenue.id.unwrap_or(aequi_core::AccountId(0));

    let mut imported = 0usize;
    let mut skipped = 0usize;

    // Wrap all inserts in a DB transaction for atomicity
    let mut db_tx = state
        .db
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("DB transaction begin failed: {e}")))?;

    for ptx in &resp.transactions {
        // Skip pending transactions
        if ptx.pending {
            skipped += 1;
            continue;
        }

        let amount_cents = ptx.amount_cents();
        if amount_cents == 0 {
            skipped += 1;
            continue;
        }

        // Duplicate detection: skip if transaction_id already imported
        let memo_marker = format!("Plaid: {}", ptx.transaction_id);
        let exists =
            sqlx::query_as::<_, (i64,)>("SELECT 1 FROM transactions WHERE memo = ? LIMIT 1")
                .bind(&memo_marker)
                .fetch_optional(&mut *db_tx)
                .await
                .map_err(|e| ApiError::Internal(format!("Duplicate check: {e}")))?;

        if exists.is_some() {
            skipped += 1;
            continue;
        }

        let date = chrono::NaiveDate::parse_from_str(&ptx.date, "%Y-%m-%d")
            .map_err(|e| ApiError::Internal(format!("Bad date '{}': {e}", ptx.date)))?;

        let description = ptx
            .merchant_name
            .as_deref()
            .unwrap_or(&ptx.name)
            .to_string();

        // Plaid: positive amount = money leaving the account (expense),
        // negative amount = money entering the account (income/revenue).
        let (debit_id, credit_id, abs_cents) = if amount_cents > 0 {
            (expense_id, checking_id, amount_cents)
        } else {
            (checking_id, revenue_id, -amount_cents)
        };

        let lines = vec![
            TransactionLine {
                account_id: debit_id,
                debit: Money::from_cents(abs_cents),
                credit: Money::from_cents(0),
                memo: None,
            },
            TransactionLine {
                account_id: credit_id,
                debit: Money::from_cents(0),
                credit: Money::from_cents(abs_cents),
                memo: None,
            },
        ];

        let tx = UnvalidatedTransaction {
            date,
            description,
            lines,
            memo: Some(memo_marker),
        };

        let validated = ValidatedTransaction::validate(tx)
            .map_err(|e| ApiError::Internal(format!("Validation error: {e}")))?;

        let row = sqlx::query_as::<_, (i64,)>(
            "INSERT INTO transactions (date, description, memo, balanced_total_cents) \
             VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(validated.date.to_string())
        .bind(&validated.description)
        .bind(&validated.memo)
        .bind(validated.balanced_total.to_cents())
        .fetch_one(&mut *db_tx)
        .await
        .map_err(|e| ApiError::Internal(format!("Insert transaction: {e}")))?;

        let tx_id = row.0;

        for line in &validated.lines {
            sqlx::query(
                "INSERT INTO transaction_lines \
                 (transaction_id, account_id, debit_cents, credit_cents, memo) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(tx_id)
            .bind(line.account_id.0)
            .bind(line.debit.to_cents())
            .bind(line.credit.to_cents())
            .bind(&line.memo)
            .execute(&mut *db_tx)
            .await
            .map_err(|e| ApiError::Internal(format!("Insert line: {e}")))?;
        }

        imported += 1;
    }

    db_tx
        .commit()
        .await
        .map_err(|e| ApiError::Internal(format!("DB commit failed: {e}")))?;

    tracing::info!(
        imported,
        skipped,
        total = resp.total_transactions,
        "Plaid sync complete"
    );

    Ok(Json(SyncResponse {
        transactions_imported: imported,
        transactions_skipped: skipped,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/plaid/link-token", post(create_link_token))
        .route("/plaid/exchange", post(exchange_public_token))
        .route("/plaid/sync", post(sync_transactions))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_link_token_request_deserializes() {
        let json = r#"{"user_id": "user_42"}"#;
        let req: CreateLinkTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.user_id.as_deref(), Some("user_42"));
    }

    #[test]
    fn create_link_token_request_optional_user_id() {
        let json = r#"{}"#;
        let req: CreateLinkTokenRequest = serde_json::from_str(json).unwrap();
        assert!(req.user_id.is_none());
    }

    #[test]
    fn exchange_response_serializes() {
        let resp = ExchangeResponse {
            item_id: "item_abc".to_string(),
            stored: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"item_id\":\"item_abc\""));
        assert!(json.contains("\"stored\":true"));
    }

    #[test]
    fn sync_request_deserializes() {
        let json = r#"{"start_date": "2026-03-01", "end_date": "2026-03-10"}"#;
        let req: SyncRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.start_date, "2026-03-01");
        assert_eq!(req.end_date, "2026-03-10");
    }

    #[test]
    fn sync_response_serializes() {
        let resp = SyncResponse {
            transactions_imported: 15,
            transactions_skipped: 3,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"transactions_imported\":15"));
        assert!(json.contains("\"transactions_skipped\":3"));
    }
}
