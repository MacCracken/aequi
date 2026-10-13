use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use aequi_import::plaid::{PlaidClient, PlaidTransaction, TransactionsSyncResult};

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

#[derive(Debug, Default, Serialize)]
pub struct SyncResponse {
    /// Review-queue batch the newly queued transactions belong to.
    pub import_batch_id: String,
    /// New bank transactions added to the review queue.
    pub transactions_queued: usize,
    /// Queue entries awaiting review that were updated because Plaid changed them.
    pub transactions_updated: usize,
    /// Queue entries awaiting review that were dropped because Plaid removed them.
    pub transactions_removed: usize,
    /// Plaid changed these after they were reviewed (posted, ignored or
    /// matched); they were left as they are. Check their ledger entries.
    pub transactions_modified_after_review: usize,
    /// Plaid removed these after they were reviewed (posted, ignored or
    /// matched), e.g. a reversed charge; they were left as they are. Check
    /// their ledger entries.
    pub transactions_removed_after_review: usize,
    /// Pending at the bank, non-USD, zero, or already queued unchanged.
    pub transactions_skipped: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn plaid_client(state: &ServerState) -> Result<&PlaidClient, ApiError> {
    state
        .plaid
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Plaid is not configured".to_string()))
}

/// Settings key holding the `/transactions/sync` cursor of a Plaid Item.
fn cursor_setting_key(item_id: &str) -> String {
    format!("plaid_cursor_{item_id}")
}

/// Review-queue statuses of entries nobody has reviewed yet. Plaid's changes
/// replace them and its removals drop them; posted, ignored and matched
/// entries are never touched.
fn awaiting_review(status: &str) -> bool {
    matches!(status, "pending" | "categorized")
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

/// POST /plaid/sync — bring the review queue up to date with Plaid.
///
/// Uses Plaid's `/transactions/sync` from the cursor stored for the Item, so
/// every addition, change and removal since the last sync arrives exactly
/// once. Nothing is posted to the ledger here: a bank feed can't tell revenue
/// from a transfer or a card payment, so every transaction is reviewed and
/// categorized like any other import before it reaches the books.
async fn sync_transactions(
    State(state): State<Arc<ServerState>>,
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
    let item_id = aequi_storage::get_setting(&state.db, "plaid_item_id")
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read item id: {e}")))?
        .unwrap_or_else(|| "default".to_string());
    let cursor_key = cursor_setting_key(&item_id);

    // One sync at a time: each must start from the cursor the last one saved.
    let _sync_guard = state.plaid_sync_lock.lock().await;

    let cursor = aequi_storage::get_setting(&state.db, &cursor_key)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read sync cursor: {e}")))?;

    let sync = client
        .transactions_sync(&access_token, cursor.as_deref())
        .await
        .map_err(|e| ApiError::Internal(format!("Plaid transactions sync error: {e}")))?;

    let result = apply_plaid_sync(&state.db, &cursor_key, &sync).await?;

    tracing::info!(
        batch = %result.import_batch_id,
        queued = result.transactions_queued,
        updated = result.transactions_updated,
        removed = result.transactions_removed,
        modified_after_review = result.transactions_modified_after_review,
        removed_after_review = result.transactions_removed_after_review,
        skipped = result.transactions_skipped,
        "Plaid sync complete"
    );

    Ok(Json(result))
}

/// Apply one `/transactions/sync` result to the review queue and store its
/// cursor. Additions and changes are queued or refresh the entry awaiting
/// review; removals drop it. All of it, cursor included, commits in one
/// database transaction: if anything fails, nothing is saved and the next
/// sync replays the same changes from the old cursor.
async fn apply_plaid_sync(
    db: &aequi_storage::DbPool,
    cursor_key: &str,
    sync: &TransactionsSyncResult,
) -> Result<SyncResponse, ApiError> {
    let mut result = SyncResponse {
        import_batch_id: format!("plaid-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };
    let account_names: HashMap<&str, &str> = sync
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.name.as_str()))
        .collect();

    let mut db_tx = db.begin_with("BEGIN IMMEDIATE").await?;

    for ptx in sync.added.iter().chain(&sync.modified) {
        match upsert_plaid_transaction(&mut db_tx, &result.import_batch_id, &account_names, ptx)
            .await?
        {
            Upsert::Queued => result.transactions_queued += 1,
            Upsert::Updated => result.transactions_updated += 1,
            Upsert::Reviewed => result.transactions_modified_after_review += 1,
            Upsert::Skipped => result.transactions_skipped += 1,
        }
    }
    for transaction_id in &sync.removed {
        match remove_plaid_transaction(&mut db_tx, transaction_id).await? {
            Removal::Removed => result.transactions_removed += 1,
            Removal::Reviewed => result.transactions_removed_after_review += 1,
            Removal::NotQueued => {}
        }
    }

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(cursor_key)
    .bind(&sync.next_cursor)
    .execute(&mut *db_tx)
    .await?;

    db_tx.commit().await?;
    Ok(result)
}

enum Upsert {
    Queued,
    Updated,
    /// Plaid changed an entry that was already reviewed; left as is.
    Reviewed,
    Skipped,
}

/// Queue an added or modified Plaid transaction, or refresh its queue entry
/// while it awaits review.
async fn upsert_plaid_transaction(
    conn: &mut SqliteConnection,
    batch_id: &str,
    account_names: &HashMap<&str, &str>,
    ptx: &PlaidTransaction,
) -> Result<Upsert, ApiError> {
    if ptx.pending {
        // Pending transactions get a new id once they post; queue that one.
        return Ok(Upsert::Skipped);
    }
    if ptx.iso_currency_code.as_deref() != Some("USD") {
        tracing::warn!(
            transaction_id = %ptx.transaction_id,
            currency = ?ptx.iso_currency_code,
            "Skipping non-USD Plaid transaction"
        );
        return Ok(Upsert::Skipped);
    }
    // Plaid reports money leaving the account as positive; the import
    // queue uses negative for money out.
    let amount_cents = match ptx.amount_cents().checked_neg() {
        Some(0) | None => return Ok(Upsert::Skipped),
        Some(c) => c,
    };
    if aequi_storage::validate_date("date", &ptx.date).is_err() {
        tracing::warn!(
            transaction_id = %ptx.transaction_id,
            date = %ptx.date,
            "Skipping Plaid transaction with invalid date"
        );
        return Ok(Upsert::Skipped);
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
    let memo = format!("Plaid account {} ({account_name})", ptx.account_id);
    let (debit_cents, credit_cents) = if amount_cents < 0 {
        (Some(-amount_cents), None)
    } else {
        (None, Some(amount_cents))
    };

    // Posted to the ledger directly by versions before the review queue.
    if posted_by_legacy_sync(conn, &ptx.transaction_id).await? {
        return Ok(Upsert::Skipped);
    }

    let existing = sqlx::query_as::<_, (i64, String, String, String, i64, Option<String>)>(
        "SELECT id, status, date, description, amount_cents, memo FROM imported_transactions \
         WHERE source_type = 'plaid' AND source_id = ? LIMIT 1",
    )
    .bind(&ptx.transaction_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some((id, status, date, old_desc, old_amount, old_memo)) = existing else {
        sqlx::query(
            "INSERT INTO imported_transactions \
             (source_type, source_id, import_batch_id, date, description, \
              amount_cents, debit_cents, credit_cents, memo, status) \
             VALUES ('plaid', ?, ?, ?, ?, ?, ?, ?, ?, 'pending')",
        )
        .bind(&ptx.transaction_id)
        .bind(batch_id)
        .bind(&ptx.date)
        .bind(&description)
        .bind(amount_cents)
        .bind(debit_cents)
        .bind(credit_cents)
        .bind(&memo)
        .execute(&mut *conn)
        .await?;
        return Ok(Upsert::Queued);
    };

    let changed = date != ptx.date
        || old_desc != description
        || old_amount != amount_cents
        || old_memo.as_deref() != Some(memo.as_str());
    if !changed {
        return Ok(Upsert::Skipped);
    }
    if !awaiting_review(&status) {
        tracing::warn!(
            import_id = id,
            status = %status,
            transaction_id = %ptx.transaction_id,
            "Plaid changed a bank transaction that was already reviewed; check its ledger entry"
        );
        return Ok(Upsert::Reviewed);
    }

    sqlx::query(
        "UPDATE imported_transactions SET date = ?, description = ?, amount_cents = ?, \
         debit_cents = ?, credit_cents = ?, memo = ? \
         WHERE id = ? AND status IN ('pending', 'categorized')",
    )
    .bind(&ptx.date)
    .bind(&description)
    .bind(amount_cents)
    .bind(debit_cents)
    .bind(credit_cents)
    .bind(&memo)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(Upsert::Updated)
}

enum Removal {
    Removed,
    /// Plaid removed an entry that was already reviewed; left as is.
    Reviewed,
    /// Never queued (pending at the bank, skipped, or before Aequi).
    NotQueued,
}

async fn remove_plaid_transaction(
    conn: &mut SqliteConnection,
    transaction_id: &str,
) -> Result<Removal, ApiError> {
    let existing = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, status FROM imported_transactions \
         WHERE source_type = 'plaid' AND source_id = ? LIMIT 1",
    )
    .bind(transaction_id)
    .fetch_optional(&mut *conn)
    .await?;

    match existing {
        Some((id, status)) if awaiting_review(&status) => {
            // A reconciliation item can still point at the entry; dismiss
            // it from the queue instead of deleting it.
            let referenced = sqlx::query_as::<_, (i64,)>(
                "SELECT id FROM reconciliation_items WHERE imported_transaction_id = ? LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .is_some();
            let sql = if referenced {
                "UPDATE imported_transactions SET status = 'ignored' WHERE id = ?"
            } else {
                "DELETE FROM imported_transactions WHERE id = ?"
            };
            sqlx::query(sql).bind(id).execute(&mut *conn).await?;
            Ok(Removal::Removed)
        }
        Some((id, status)) => {
            tracing::warn!(
                import_id = id,
                status = %status,
                transaction_id,
                "Plaid removed a bank transaction that was already reviewed; check its ledger entry"
            );
            Ok(Removal::Reviewed)
        }
        None if posted_by_legacy_sync(conn, transaction_id).await? => {
            tracing::warn!(
                transaction_id,
                "Plaid removed a bank transaction that is in the ledger; check its entry"
            );
            Ok(Removal::Reviewed)
        }
        None => Ok(Removal::NotQueued),
    }
}

/// Versions before the review queue posted Plaid transactions straight to
/// the ledger with this memo.
async fn posted_by_legacy_sync(
    conn: &mut SqliteConnection,
    transaction_id: &str,
) -> Result<bool, ApiError> {
    let legacy = sqlx::query_as::<_, (i64,)>("SELECT id FROM transactions WHERE memo = ? LIMIT 1")
        .bind(format!("Plaid: {transaction_id}"))
        .fetch_optional(&mut *conn)
        .await?;
    Ok(legacy.is_some())
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
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use axum::http::{Method, StatusCode};
    use serde_json::{Value, json};

    use super::*;
    use crate::routes::tests::{call, mock_server, test_state};

    const CURSOR_KEY: &str = "plaid_cursor_item_1";

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
    fn sync_response_serializes() {
        let resp = SyncResponse {
            import_batch_id: "plaid-1".to_string(),
            transactions_queued: 15,
            transactions_skipped: 3,
            transactions_removed_after_review: 1,
            ..Default::default()
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"transactions_queued\":15"));
        assert!(json.contains("\"transactions_skipped\":3"));
        assert!(json.contains("\"transactions_removed_after_review\":1"));
    }

    fn tx_json(id: &str, amount: f64, date: &str, pending: bool) -> Value {
        json!({
            "transaction_id": id,
            "name": "Some Merchant",
            "amount": amount,
            "date": date,
            "category": null,
            "account_id": "acc_1",
            "pending": pending,
            "iso_currency_code": "USD",
            "merchant_name": null
        })
    }

    fn ptx(id: &str, amount: f64, date: &str, pending: bool) -> PlaidTransaction {
        serde_json::from_value(tx_json(id, amount, date, pending)).unwrap()
    }

    fn accounts_json() -> Value {
        json!([{
            "account_id": "acc_1",
            "name": "Business Checking",
            "type": "depository",
            "subtype": "checking",
            "balances": {"available": null, "current": null, "limit": null, "iso_currency_code": "USD"}
        }])
    }

    fn sync_result(
        added: Vec<PlaidTransaction>,
        modified: Vec<PlaidTransaction>,
        removed: &[&str],
        next_cursor: &str,
    ) -> TransactionsSyncResult {
        TransactionsSyncResult {
            accounts: serde_json::from_value(accounts_json()).unwrap(),
            added,
            modified,
            removed: removed.iter().map(|s| s.to_string()).collect(),
            next_cursor: next_cursor.to_string(),
        }
    }

    async fn queue_rows(db: &aequi_storage::DbPool) -> Vec<(String, i64, String, Option<String>)> {
        sqlx::query_as(
            "SELECT source_id, amount_cents, status, memo FROM imported_transactions ORDER BY source_id",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    async fn stored_cursor(db: &aequi_storage::DbPool) -> Option<String> {
        aequi_storage::get_setting(db, CURSOR_KEY).await.unwrap()
    }

    async fn set_status(db: &aequi_storage::DbPool, source_id: &str, status: &str) {
        sqlx::query("UPDATE imported_transactions SET status = ? WHERE source_id = ?")
            .bind(status)
            .bind(source_id)
            .execute(db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn sync_queues_for_review_instead_of_posting() {
        let state = test_state(None).await;
        let db = &state.db;
        let added = vec![
            ptx("t_out", 42.50, "2026-03-02", false), // card purchase / transfer out
            ptx("t_in", -1000.00, "2026-03-03", false), // deposit
            ptx("t_pending", 5.00, "2026-03-04", true),
        ];
        let r = apply_plaid_sync(
            db,
            CURSOR_KEY,
            &sync_result(added.clone(), vec![], &[], "c1"),
        )
        .await
        .unwrap();
        assert_eq!(r.transactions_queued, 2);
        assert_eq!(r.transactions_skipped, 1);
        assert_eq!(stored_cursor(db).await.as_deref(), Some("c1"));

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

        // Replaying the same additions (e.g. a sync from an empty cursor)
        // queues nothing new.
        let again = apply_plaid_sync(db, CURSOR_KEY, &sync_result(added, vec![], &[], "c2"))
            .await
            .unwrap();
        assert_eq!(again.transactions_queued, 0);
        assert_eq!(queue_rows(db).await.len(), 2);
        assert_eq!(stored_cursor(db).await.as_deref(), Some("c2"));
    }

    #[tokio::test]
    async fn sync_applies_modifications_and_removals_exactly() {
        let state = test_state(None).await;
        let db = &state.db;
        let first = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|id| ptx(id, 10.00, "2026-03-02", false))
            .collect();
        apply_plaid_sync(db, CURSOR_KEY, &sync_result(first, vec![], &[], "c1"))
            .await
            .unwrap();

        // "c" was posted and "d" ignored; "e" was categorized by a rule but
        // still awaits review.
        set_status(db, "c", "posted").await;
        set_status(db, "d", "ignored").await;
        set_status(db, "e", "categorized").await;

        let modified = vec![
            ptx("a", 12.00, "2026-03-02", false),
            ptx("c", 15.00, "2026-03-02", false),
            ptx("e", 11.00, "2026-03-03", false),
            // Never queued before (e.g. was zero): queued now.
            ptx("f", 3.00, "2026-03-05", false),
        ];
        let r = apply_plaid_sync(
            db,
            CURSOR_KEY,
            &sync_result(vec![], modified, &["b", "c", "d", "never_queued"], "c2"),
        )
        .await
        .unwrap();
        assert_eq!(r.transactions_updated, 2, "a and e");
        assert_eq!(r.transactions_queued, 1, "f");
        assert_eq!(r.transactions_modified_after_review, 1, "c");
        assert_eq!(r.transactions_removed, 1, "b");
        assert_eq!(r.transactions_removed_after_review, 2, "c and d");

        let rows: Vec<_> = queue_rows(db)
            .await
            .into_iter()
            .map(|r| (r.0, r.1, r.2))
            .collect();
        let expected = [
            ("a", -1_200, "pending"),
            ("c", -1_000, "posted"),
            ("d", -1_000, "ignored"),
            ("e", -1_100, "categorized"),
            ("f", -300, "pending"),
        ];
        assert_eq!(
            rows,
            expected
                .iter()
                .map(|(id, c, s)| (id.to_string(), *c, s.to_string()))
                .collect::<Vec<_>>()
        );

        // Nothing is inferred from what a sync doesn't mention.
        let r = apply_plaid_sync(db, CURSOR_KEY, &sync_result(vec![], vec![], &[], "c3"))
            .await
            .unwrap();
        assert_eq!(r.transactions_removed, 0);
        assert_eq!(queue_rows(db).await.len(), 5);
    }

    #[tokio::test]
    async fn removed_entry_used_by_reconciliation_is_dismissed_not_deleted() {
        let state = test_state(None).await;
        let db = &state.db;
        let added = vec![ptx("a", 10.00, "2026-03-02", false)];
        apply_plaid_sync(db, CURSOR_KEY, &sync_result(added, vec![], &[], "c1"))
            .await
            .unwrap();
        let (import_id,): (i64,) = sqlx::query_as("SELECT id FROM imported_transactions")
            .fetch_one(db)
            .await
            .unwrap();
        let checking = aequi_storage::get_account_by_code(db, "1000")
            .await
            .unwrap()
            .unwrap()
            .id
            .unwrap();
        sqlx::query(
            "INSERT INTO reconciliation_sessions (account_id, start_date, end_date, statement_balance_cents) \
             VALUES (?, '2026-03-01', '2026-03-31', 0)",
        )
        .bind(checking.0)
        .execute(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO reconciliation_items (session_id, imported_transaction_id, match_type) \
             VALUES (1, ?, 'unmatched')",
        )
        .bind(import_id)
        .execute(db)
        .await
        .unwrap();

        let r = apply_plaid_sync(db, CURSOR_KEY, &sync_result(vec![], vec![], &["a"], "c2"))
            .await
            .unwrap();
        assert_eq!(r.transactions_removed, 1);
        let rows = queue_rows(db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "ignored");
    }

    #[tokio::test]
    async fn failed_sync_saves_neither_queue_changes_nor_cursor() {
        let state = test_state(None).await;
        let db = &state.db;
        let added = vec![ptx("a", 10.00, "2026-03-02", false)];
        apply_plaid_sync(db, CURSOR_KEY, &sync_result(added, vec![], &[], "c1"))
            .await
            .unwrap();

        // Make the removal step fail after "b" has been inserted.
        sqlx::query("DROP TABLE reconciliation_items")
            .execute(db)
            .await
            .unwrap();
        let added = vec![ptx("b", 20.00, "2026-03-03", false)];
        let err = apply_plaid_sync(db, CURSOR_KEY, &sync_result(added, vec![], &["a"], "c2")).await;
        assert!(err.is_err());

        let ids: Vec<_> = queue_rows(db).await.into_iter().map(|r| r.0).collect();
        assert_eq!(ids, ["a"]);
        assert_eq!(stored_cursor(db).await.as_deref(), Some("c1"));
    }

    #[tokio::test]
    async fn sync_skips_transactions_already_posted_by_old_versions() {
        let state = test_state(None).await;
        let db = &state.db;
        let account = |code: &'static str| async move {
            aequi_storage::get_account_by_code(db, code)
                .await
                .unwrap()
                .unwrap()
                .id
                .unwrap()
        };
        let legacy = aequi_core::UnvalidatedTransaction {
            date: chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            description: "x".to_string(),
            lines: vec![
                aequi_core::TransactionLine::debit(
                    account("5900").await,
                    aequi_core::Money::from_cents(100),
                    None,
                ),
                aequi_core::TransactionLine::credit(
                    account("1000").await,
                    aequi_core::Money::from_cents(100),
                    None,
                ),
            ],
            memo: Some("Plaid: legacy".to_string()),
        };
        aequi_storage::create_ledger_transaction(db, legacy, None)
            .await
            .unwrap();
        let added = vec![ptx("legacy", 1.00, "2026-03-02", false)];
        let r = apply_plaid_sync(db, CURSOR_KEY, &sync_result(added, vec![], &[], "c1"))
            .await
            .unwrap();
        assert_eq!(r.transactions_queued, 0);
        assert_eq!(r.transactions_skipped, 1);

        let r = apply_plaid_sync(
            db,
            CURSOR_KEY,
            &sync_result(vec![], vec![], &["legacy"], "c2"),
        )
        .await
        .unwrap();
        assert_eq!(r.transactions_removed_after_review, 1);
    }

    /// A stand-in for Plaid's `/transactions/sync` that answers with the
    /// scripted responses in order and records each request body.
    #[derive(Clone, Default)]
    struct MockPlaid {
        responses: Arc<Mutex<VecDeque<(StatusCode, Value)>>>,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    impl MockPlaid {
        fn respond(&self, status: StatusCode, body: Value) {
            self.responses.lock().unwrap().push_back((status, body));
        }

        fn page(&self, added: Vec<Value>, removed: &[&str], next_cursor: &str, has_more: bool) {
            let removed: Vec<_> = removed
                .iter()
                .map(|id| json!({"transaction_id": id, "account_id": "acc_1"}))
                .collect();
            self.respond(
                StatusCode::OK,
                json!({
                    "accounts": accounts_json(),
                    "added": added,
                    "modified": [],
                    "removed": removed,
                    "next_cursor": next_cursor,
                    "has_more": has_more,
                    "request_id": "req"
                }),
            );
        }

        fn cursors_sent(&self) -> Vec<Option<String>> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.get("cursor").and_then(Value::as_str).map(str::to_string))
                .collect()
        }

        async fn start(&self) -> String {
            let mock = self.clone();
            let app = axum::Router::new().route(
                "/transactions/sync",
                post(move |Json(body): Json<Value>| async move {
                    mock.requests.lock().unwrap().push(body);
                    let (status, resp) = mock
                        .responses
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("unexpected Plaid request");
                    (status, Json(resp))
                }),
            );
            mock_server(app).await
        }
    }

    async fn plaid_state(base_url: &str) -> Arc<ServerState> {
        let mut state = test_state(None).await;
        let config: aequi_import::plaid::PlaidConfig = serde_json::from_value(
            json!({"client_id": "cid", "secret": "sec", "environment": "sandbox"}),
        )
        .unwrap();
        Arc::get_mut(&mut state).unwrap().plaid =
            Some(PlaidClient::with_base_url(config, base_url));
        aequi_storage::set_setting(&state.db, "plaid_access_token", "access-sandbox-1")
            .await
            .unwrap();
        aequi_storage::set_setting(&state.db, "plaid_item_id", "item_1")
            .await
            .unwrap();
        state
    }

    #[tokio::test]
    async fn sync_endpoint_follows_cursor_across_syncs() {
        let mock = MockPlaid::default();
        // First sync: two pages, interrupted once by a mutation, so the loop
        // restarts from the beginning.
        mock.page(
            vec![tx_json("stale", 1.0, "2026-03-01", false)],
            &[],
            "p1",
            true,
        );
        mock.respond(
            StatusCode::BAD_REQUEST,
            json!({
                "error_type": "TRANSACTIONS_ERROR",
                "error_code": "TRANSACTIONS_SYNC_MUTATION_DURING_PAGINATION",
                "error_message": "Underlying transaction data changed since last page was fetched",
                "request_id": "req"
            }),
        );
        mock.page(
            vec![tx_json("a", 10.0, "2026-03-02", false)],
            &[],
            "p1b",
            true,
        );
        mock.page(
            vec![tx_json("b", 20.0, "2026-03-03", false)],
            &[],
            "c1",
            false,
        );
        let state = plaid_state(&mock.start().await).await;

        let (status, body) = call(&state, Method::POST, "/api/v1/plaid/sync", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["transactions_queued"], 2);
        assert_eq!(
            mock.cursors_sent(),
            [None, Some("p1".into()), None, Some("p1b".into())]
        );
        assert_eq!(stored_cursor(&state.db).await.as_deref(), Some("c1"));
        let ids: Vec<_> = queue_rows(&state.db)
            .await
            .into_iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        {
            let requests = mock.requests.lock().unwrap();
            assert_eq!(requests[0]["access_token"], "access-sandbox-1");
            assert_eq!(requests[0]["client_id"], "cid");
        }

        // Second sync resumes from the stored cursor; a request body from an
        // older client is ignored.
        mock.page(vec![], &["a"], "c2", false);
        let (status, body) = call(
            &state,
            Method::POST,
            "/api/v1/plaid/sync",
            Some(json!({"start_date": "2026-03-01", "end_date": "2026-03-31"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["transactions_removed"], 1);
        assert_eq!(mock.cursors_sent().last().unwrap().as_deref(), Some("c1"));
        assert_eq!(stored_cursor(&state.db).await.as_deref(), Some("c2"));
        let ids: Vec<_> = queue_rows(&state.db)
            .await
            .into_iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(ids, ["b"]);
    }

    #[tokio::test]
    async fn plaid_error_keeps_the_cursor() {
        let mock = MockPlaid::default();
        mock.respond(
            StatusCode::BAD_REQUEST,
            json!({
                "error_type": "ITEM_ERROR",
                "error_code": "ITEM_LOGIN_REQUIRED",
                "error_message": "the login details of this item have changed",
                "request_id": "req"
            }),
        );
        let state = plaid_state(&mock.start().await).await;
        aequi_storage::set_setting(&state.db, CURSOR_KEY, "c7")
            .await
            .unwrap();

        let (status, body) = call(&state, Method::POST, "/api/v1/plaid/sync", None).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!body.to_string().contains("access-sandbox-1"));
        assert_eq!(mock.cursors_sent(), [Some("c7".to_string())]);
        assert_eq!(stored_cursor(&state.db).await.as_deref(), Some("c7"));
    }

    #[tokio::test]
    async fn sync_requires_configuration_and_a_linked_item() {
        let state = test_state(None).await;
        let (status, body) = call(&state, Method::POST, "/api/v1/plaid/sync", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("not configured"));

        let state = plaid_state("http://127.0.0.1:1").await;
        sqlx::query("DELETE FROM settings WHERE key = 'plaid_access_token'")
            .execute(&state.db)
            .await
            .unwrap();
        let (status, body) = call(&state, Method::POST, "/api/v1/plaid/sync", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("Link"));
    }
}
