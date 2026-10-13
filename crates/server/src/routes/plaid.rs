use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_import::plaid::{PlaidAccount, PlaidClient, PlaidTransaction};

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

#[derive(Debug, Default, Serialize)]
pub struct SyncResponse {
    /// Review-queue batch the newly queued transactions belong to.
    pub import_batch_id: String,
    /// New bank transactions added to the review queue.
    pub transactions_queued: usize,
    /// Still-pending queue entries updated because Plaid changed them.
    pub transactions_updated: usize,
    /// Still-pending queue entries dropped because Plaid no longer reports them.
    pub transactions_removed: usize,
    /// Pending at the bank, non-USD, zero, or already queued/reviewed.
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

/// POST /plaid/sync — fetch transactions from Plaid into the import review queue.
///
/// Nothing is posted to the ledger here: a bank feed can't tell revenue from
/// a transfer or a card payment, so every transaction is reviewed and
/// categorized like any other import before it reaches the books.
async fn sync_transactions(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<SyncRequest>,
) -> Result<Json<SyncResponse>, ApiError> {
    aequi_storage::validate_date("start_date", &body.start_date)?;
    aequi_storage::validate_date("end_date", &body.end_date)?;
    if body.start_date > body.end_date {
        return Err(ApiError::BadRequest(
            "start_date must not be after end_date".to_string(),
        ));
    }

    let client = plaid_client(&state)?;

    let access_token = aequi_storage::get_setting(&state.db, "plaid_access_token")
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read access token: {e}")))?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "No Plaid access token stored. Complete the Link flow first.".to_string(),
            )
        })?;

    // One sync at a time, so concurrent requests can't queue duplicates.
    let _sync_guard = state.plaid_sync_lock.lock().await;

    let resp = client
        .get_transactions(&access_token, &body.start_date, &body.end_date)
        .await
        .map_err(|e| ApiError::Internal(format!("Plaid transactions error: {e}")))?;

    let result = queue_plaid_transactions(
        &state.db,
        &resp.accounts,
        &resp.transactions,
        &body.start_date,
        &body.end_date,
    )
    .await?;

    tracing::info!(
        batch = %result.import_batch_id,
        queued = result.transactions_queued,
        updated = result.transactions_updated,
        removed = result.transactions_removed,
        skipped = result.transactions_skipped,
        total = resp.total_transactions,
        "Plaid sync complete"
    );

    Ok(Json(result))
}

fn account_memo_prefix(plaid_account_id: &str) -> String {
    format!("Plaid account {plaid_account_id} (")
}

/// Reconcile one `/transactions/get` result for `start..=end` with the review
/// queue: queue new posted transactions, refresh still-pending entries Plaid
/// changed, and drop still-pending entries in the range Plaid no longer returns.
/// Entries a user already matched or categorized are never touched.
async fn queue_plaid_transactions(
    db: &aequi_storage::DbPool,
    accounts: &[PlaidAccount],
    transactions: &[PlaidTransaction],
    start: &str,
    end: &str,
) -> Result<SyncResponse, ApiError> {
    let mut result = SyncResponse {
        import_batch_id: format!("plaid-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };
    let account_names: HashMap<&str, &str> = accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.name.as_str()))
        .collect();
    let returned: HashSet<&str> = transactions
        .iter()
        .map(|t| t.transaction_id.as_str())
        .collect();

    for ptx in transactions {
        if ptx.pending {
            // Pending transactions get a new id once they post; queue that one.
            result.transactions_skipped += 1;
            continue;
        }
        if ptx.iso_currency_code.as_deref() != Some("USD") {
            tracing::warn!(
                transaction_id = %ptx.transaction_id,
                currency = ?ptx.iso_currency_code,
                "Skipping non-USD Plaid transaction"
            );
            result.transactions_skipped += 1;
            continue;
        }
        // Plaid reports money leaving the account as positive; the import
        // queue uses negative for money out.
        let amount_cents = match ptx.amount_cents().checked_neg() {
            Some(0) | None => {
                result.transactions_skipped += 1;
                continue;
            }
            Some(c) => c,
        };
        if aequi_storage::validate_date("date", &ptx.date).is_err() {
            tracing::warn!(
                transaction_id = %ptx.transaction_id,
                date = %ptx.date,
                "Skipping Plaid transaction with invalid date"
            );
            result.transactions_skipped += 1;
            continue;
        }

        let description = ptx
            .merchant_name
            .as_deref()
            .unwrap_or(&ptx.name)
            .to_string();
        let account_name = account_names
            .get(ptx.account_id.as_str())
            .copied()
            .unwrap_or("unknown");
        let memo = format!("{}{account_name})", account_memo_prefix(&ptx.account_id));
        let (debit_cents, credit_cents) = if amount_cents < 0 {
            (Some(-amount_cents), None)
        } else {
            (None, Some(amount_cents))
        };

        // Posted to the ledger directly by versions before the review queue.
        let legacy =
            sqlx::query_as::<_, (i64,)>("SELECT id FROM transactions WHERE memo = ? LIMIT 1")
                .bind(format!("Plaid: {}", ptx.transaction_id))
                .fetch_optional(db)
                .await?;
        if legacy.is_some() {
            result.transactions_skipped += 1;
            continue;
        }

        let existing = sqlx::query_as::<_, (i64, String, String, String, i64, Option<String>)>(
            "SELECT id, status, date, description, amount_cents, memo FROM imported_transactions \
             WHERE source_type = 'plaid' AND source_id = ? LIMIT 1",
        )
        .bind(&ptx.transaction_id)
        .fetch_optional(db)
        .await?;

        match existing {
            None => {
                aequi_storage::insert_imported_transaction(
                    db,
                    &aequi_storage::ImportedTransaction {
                        id: 0,
                        source_type: "plaid".to_string(),
                        source_id: Some(ptx.transaction_id.clone()),
                        import_batch_id: result.import_batch_id.clone(),
                        date: ptx.date.clone(),
                        description,
                        amount_cents,
                        debit_cents,
                        credit_cents,
                        memo: Some(memo),
                        matched_transaction_id: None,
                        category_rule_id: None,
                        status: "pending".to_string(),
                        created_at: String::new(),
                    },
                )
                .await?;
                result.transactions_queued += 1;
            }
            Some((id, status, date, old_desc, old_amount, old_memo))
                if status == "pending"
                    && (date != ptx.date
                        || old_desc != description
                        || old_amount != amount_cents
                        || old_memo.as_deref() != Some(memo.as_str())) =>
            {
                sqlx::query(
                    "UPDATE imported_transactions SET date = ?, description = ?, amount_cents = ?, \
                     debit_cents = ?, credit_cents = ?, memo = ? WHERE id = ? AND status = 'pending'",
                )
                .bind(&ptx.date)
                .bind(&description)
                .bind(amount_cents)
                .bind(debit_cents)
                .bind(credit_cents)
                .bind(&memo)
                .bind(id)
                .execute(db)
                .await?;
                result.transactions_updated += 1;
            }
            Some(_) => result.transactions_skipped += 1,
        }
    }

    // Removed: still-pending entries for these accounts dated in the synced
    // range that Plaid no longer returns. If one was dropped by mistake, the
    // next sync of its date re-queues it.
    let candidates = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(
        "SELECT id, source_id, memo FROM imported_transactions \
         WHERE source_type = 'plaid' AND status = 'pending' AND date >= ? AND date <= ?",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    for (id, source_id, memo) in candidates {
        let Some(source_id) = source_id else { continue };
        let memo = memo.unwrap_or_default();
        let same_item = account_names
            .keys()
            .any(|acct| memo.starts_with(&account_memo_prefix(acct)));
        if same_item && !returned.contains(source_id.as_str()) {
            sqlx::query("DELETE FROM imported_transactions WHERE id = ? AND status = 'pending'")
                .bind(id)
                .execute(db)
                .await?;
            result.transactions_removed += 1;
        }
    }

    Ok(result)
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
            import_batch_id: "plaid-1".to_string(),
            transactions_queued: 15,
            transactions_skipped: 3,
            ..Default::default()
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"transactions_queued\":15"));
        assert!(json.contains("\"transactions_skipped\":3"));
    }

    fn ptx(id: &str, amount: f64, date: &str, pending: bool) -> PlaidTransaction {
        serde_json::from_value(serde_json::json!({
            "transaction_id": id,
            "name": "Some Merchant",
            "amount": amount,
            "date": date,
            "category": null,
            "account_id": "acc_1",
            "pending": pending,
            "iso_currency_code": "USD",
            "merchant_name": null
        }))
        .unwrap()
    }

    fn accounts() -> Vec<PlaidAccount> {
        serde_json::from_value(serde_json::json!([{
            "account_id": "acc_1",
            "name": "Business Checking",
            "type": "depository",
            "subtype": "checking",
            "balances": {"available": null, "current": null, "limit": null, "iso_currency_code": "USD"}
        }]))
        .unwrap()
    }

    async fn queue_rows(db: &aequi_storage::DbPool) -> Vec<(String, i64, String, Option<String>)> {
        sqlx::query_as(
            "SELECT source_id, amount_cents, status, memo FROM imported_transactions ORDER BY source_id",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn sync_queues_for_review_instead_of_posting() {
        let state = crate::routes::tests::test_state(None).await;
        let db = &state.db;
        let txs = vec![
            ptx("t_out", 42.50, "2026-03-02", false), // card purchase / transfer out
            ptx("t_in", -1000.00, "2026-03-03", false), // deposit
            ptx("t_pending", 5.00, "2026-03-04", true),
        ];
        let r = queue_plaid_transactions(db, &accounts(), &txs, "2026-03-01", "2026-03-31")
            .await
            .unwrap();
        assert_eq!(r.transactions_queued, 2);
        assert_eq!(r.transactions_skipped, 1);

        // Nothing reached the ledger.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(n, 0);

        let rows = queue_rows(db).await;
        assert_eq!(rows.len(), 2);
        // Money out is negative in the queue (Plaid reports it positive).
        assert_eq!((rows[0].0.as_str(), rows[0].1), ("t_in", 100_000));
        assert_eq!((rows[1].0.as_str(), rows[1].1), ("t_out", -4_250));
        assert!(rows.iter().all(|r| r.2 == "pending"));
        assert!(rows[1].3.as_deref().unwrap().contains("Business Checking"));
        assert!(rows[1].3.as_deref().unwrap().contains("acc_1"));

        // Re-syncing the same data queues nothing new.
        let again = queue_plaid_transactions(db, &accounts(), &txs, "2026-03-01", "2026-03-31")
            .await
            .unwrap();
        assert_eq!(again.transactions_queued, 0);
        assert_eq!(queue_rows(db).await.len(), 2);
    }

    #[tokio::test]
    async fn sync_updates_modified_and_drops_removed_pending_entries() {
        let state = crate::routes::tests::test_state(None).await;
        let db = &state.db;
        let first = vec![
            ptx("a", 10.00, "2026-03-02", false),
            ptx("b", 20.00, "2026-03-03", false),
            ptx("c", 30.00, "2026-03-04", false),
        ];
        queue_plaid_transactions(db, &accounts(), &first, "2026-03-01", "2026-03-31")
            .await
            .unwrap();

        // A user already reviewed "c": later syncs must leave it alone.
        sqlx::query(
            "UPDATE imported_transactions SET status = 'categorized' WHERE source_id = 'c'",
        )
        .execute(db)
        .await
        .unwrap();

        // Plaid now reports "a" with a new amount and no longer returns "b" or "c".
        let second = vec![ptx("a", 12.00, "2026-03-02", false)];
        let r = queue_plaid_transactions(db, &accounts(), &second, "2026-03-01", "2026-03-31")
            .await
            .unwrap();
        assert_eq!(r.transactions_updated, 1);
        assert_eq!(r.transactions_removed, 1);

        let rows = queue_rows(db).await;
        let ids: Vec<_> = rows.iter().map(|r| (r.0.as_str(), r.1)).collect();
        assert_eq!(ids, vec![("a", -1_200), ("c", -3_000)]);
    }

    #[tokio::test]
    async fn sync_skips_transactions_already_posted_by_old_versions() {
        let state = crate::routes::tests::test_state(None).await;
        let db = &state.db;
        sqlx::query(
            "INSERT INTO transactions (date, description, memo, balanced_total_cents) \
             VALUES ('2026-03-02', 'x', 'Plaid: legacy', 100)",
        )
        .execute(db)
        .await
        .unwrap();
        let txs = vec![ptx("legacy", 1.00, "2026-03-02", false)];
        let r = queue_plaid_transactions(db, &accounts(), &txs, "2026-03-01", "2026-03-31")
            .await
            .unwrap();
        assert_eq!(r.transactions_queued, 0);
        assert_eq!(r.transactions_skipped, 1);
    }
}
