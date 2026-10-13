use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

use crate::error::ApiError;
use crate::state::ServerState;

/// Maximum age (either direction) of a signed webhook, as in Stripe's libraries.
const SIGNATURE_TOLERANCE_SECS: i64 = 300;

// ── Stripe event types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct StripeEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    data: StripeEventData,
    created: i64,
}

#[derive(Debug, Deserialize)]
struct StripeEventData {
    object: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct WebhookResponse {
    processed: bool,
    event_id: String,
    transaction_id: Option<i64>,
    message: String,
}

impl WebhookResponse {
    fn skipped(event: &StripeEvent, message: impl Into<String>) -> Self {
        Self {
            processed: false,
            event_id: event.id.clone(),
            transaction_id: None,
            message: message.into(),
        }
    }
}

// ── Signature verification ─────────────────────────────────────────────────

/// A parsed `Stripe-Signature` header: `t=<unix time>,v1=<hex>[,v1=<hex>…]`.
#[derive(Debug)]
struct SignatureHeader<'a> {
    /// The timestamp exactly as sent; it is what Stripe signed.
    timestamp_raw: &'a str,
    timestamp: i64,
    v1: Vec<&'a str>,
}

/// Parse the header once so the HMAC and the replay check use the same `t`.
/// Exactly one all-digit `t` is required; several `v1` entries are allowed
/// (Stripe sends one per active secret during rotation).
fn parse_signature_header(header: &str) -> Result<SignatureHeader<'_>, &'static str> {
    let mut timestamp_raw = None;
    let mut v1 = Vec::new();

    for part in header.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        match key {
            "t" => {
                if timestamp_raw.replace(value).is_some() {
                    return Err("multiple timestamps in signature header");
                }
            }
            "v1" => v1.push(value),
            _ => {}
        }
    }

    let timestamp_raw = timestamp_raw.ok_or("missing timestamp in signature header")?;
    if timestamp_raw.is_empty() || !timestamp_raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err("non-numeric timestamp in signature header");
    }
    let timestamp = timestamp_raw
        .parse::<i64>()
        .map_err(|_| "timestamp out of range")?;
    if v1.is_empty() {
        return Err("missing v1 signature");
    }

    Ok(SignatureHeader {
        timestamp_raw,
        timestamp,
        v1,
    })
}

/// HMAC-SHA256(secret, "{timestamp}.{payload}") as lowercase hex.
fn compute_signature(secret: &str, timestamp: &str, payload: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    // HMAC via manual SHA-256 (key XOR pad approach)
    let key = secret.as_bytes();
    let block_size = 64;

    let normalized_key = if key.len() > block_size {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.finalize().to_vec()
    } else {
        key.to_vec()
    };

    let mut ipad = vec![0x36u8; block_size];
    let mut opad = vec![0x5cu8; block_size];
    for (i, &b) in normalized_key.iter().enumerate() {
        ipad[i] ^= b;
        opad[i] ^= b;
    }

    let mut inner_hasher = Sha256::new();
    inner_hasher.update(&ipad);
    inner_hasher.update(timestamp.as_bytes());
    inner_hasher.update(b".");
    inner_hasher.update(payload);
    let inner_hash = inner_hasher.finalize();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&opad);
    outer_hasher.update(inner_hash);
    hex::encode(outer_hasher.finalize())
}

/// Verify the signature and the replay window against `now` (unix seconds).
fn verify_stripe_signature(
    payload: &[u8],
    sig_header: &str,
    secret: &str,
    now: i64,
) -> Result<(), &'static str> {
    let header = parse_signature_header(sig_header)?;

    if (now - header.timestamp).abs() > SIGNATURE_TOLERANCE_SECS {
        return Err("signature timestamp outside tolerance");
    }

    let expected = compute_signature(secret, header.timestamp_raw, payload);
    if header
        .v1
        .iter()
        .any(|sig| super::constant_time_eq(sig.as_bytes(), expected.as_bytes()))
    {
        Ok(())
    } else {
        Err("no matching v1 signature")
    }
}

// ── Account mapping ────────────────────────────────────────────────────────

/// Map Stripe event types to debit/credit account codes.
fn stripe_account_mapping(event_type: &str) -> Option<(&'static str, &'static str, &'static str)> {
    // Returns (description_prefix, debit_account, credit_account)
    match event_type {
        // Payout: money moves from Stripe to bank
        "payout.paid" => Some(("Stripe payout", "1000", "1030")),
        // Charge succeeded: revenue recognized
        "charge.succeeded" => Some(("Stripe charge", "1030", "4000")),
        // Refund: reverse the revenue
        "charge.refunded" => Some(("Stripe refund", "4000", "1030")),
        // Fee deducted (from balance transaction)
        "balance.available" => None, // handled via payout details
        _ => None,
    }
}

fn event_memo(event_id: &str) -> String {
    format!("Stripe event: {event_id}")
}

// ── Webhook handler ────────────────────────────────────────────────────────

async fn handle_webhook(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<WebhookResponse>, ApiError> {
    // SECURITY: Always require webhook secret — reject if not configured
    let secret = state
        .stripe_webhook_secret
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("Stripe webhook not configured".to_string()))?;

    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Err(reason) = verify_stripe_signature(&body, sig, secret, chrono::Utc::now().timestamp())
    {
        tracing::warn!(reason, "Rejected Stripe webhook");
        return Err(ApiError::Unauthorized);
    }

    let event: StripeEvent = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("Invalid event: {e}")))?;

    tracing::info!(
        event_id = %event.id,
        event_type = %event.event_type,
        "Processing Stripe webhook"
    );

    process_event(&state.db, &event).await.map(Json)
}

/// Book a verified Stripe event. Idempotent per event id, so Stripe's
/// retries (and duplicate deliveries) never post twice.
async fn process_event(
    db: &aequi_storage::DbPool,
    event: &StripeEvent,
) -> Result<WebhookResponse, ApiError> {
    let Some((desc_prefix, debit_code, credit_code)) = stripe_account_mapping(&event.event_type)
    else {
        return Ok(WebhookResponse::skipped(
            event,
            format!("Event type '{}' not mapped", event.event_type),
        ));
    };
    let obj = &event.data.object;

    // The ledger is in USD; booking ¥5000 as $50.00 would be wrong.
    let currency = obj.get("currency").and_then(|v| v.as_str()).unwrap_or("");
    if !currency.eq_ignore_ascii_case("usd") {
        tracing::warn!(
            event_id = %event.id,
            currency,
            "Skipping non-USD Stripe event; record it manually"
        );
        return Ok(WebhookResponse::skipped(
            event,
            format!("Currency '{currency}' not supported, skipped"),
        ));
    }

    let is_refund = event.event_type == "charge.refunded";
    // For refunds Stripe sends the charge; `amount_refunded` is cumulative.
    let amount_field = if is_refund {
        "amount_refunded"
    } else {
        "amount"
    };
    let event_amount = obj.get(amount_field).and_then(|v| v.as_i64()).unwrap_or(0);
    if event_amount < 0 {
        return Err(ApiError::BadRequest("Negative amount".to_string()));
    }

    let stripe_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or(&event.id);
    let description = format!("{desc_prefix}: {stripe_id}");

    // Extract date from event timestamp (local calendar date)
    let date = chrono::DateTime::from_timestamp(event.created, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive());

    let account_id = |code: &'static str| async move {
        aequi_storage::get_account_by_code(db, code)
            .await?
            .and_then(|a| a.id)
            .ok_or_else(|| ApiError::Internal(format!("Account {code} not found")))
    };
    let debit_id = account_id(debit_code).await?;
    let credit_id = account_id(credit_code).await?;

    // IMMEDIATE takes the write lock up front, so the duplicate checks below
    // can't race with a concurrent delivery of the same event.
    let mut db_tx = db.begin_with("BEGIN IMMEDIATE").await?;

    let memo = event_memo(&event.id);
    let existing =
        sqlx::query_as::<_, (i64,)>("SELECT id FROM transactions WHERE memo = ? LIMIT 1")
            .bind(&memo)
            .fetch_optional(&mut *db_tx)
            .await?;
    if let Some((tx_id,)) = existing {
        tracing::info!(event_id = %event.id, transaction_id = tx_id, "Duplicate Stripe event ignored");
        return Ok(WebhookResponse {
            transaction_id: Some(tx_id),
            ..WebhookResponse::skipped(event, "Event already processed")
        });
    }

    let amount_cents = if is_refund {
        // Partial refunds arrive as successive charge.refunded events with a
        // growing amount_refunded; book only what isn't booked yet.
        let (already,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(balanced_total_cents), 0) FROM transactions WHERE description = ?",
        )
        .bind(&description)
        .fetch_one(&mut *db_tx)
        .await?;
        event_amount - already
    } else {
        event_amount
    };

    if amount_cents <= 0 {
        return Ok(WebhookResponse::skipped(
            event,
            if is_refund {
                "Refund already recorded"
            } else {
                "Zero amount, skipped"
            },
        ));
    }

    // Handle Stripe fees as a separate line if present
    let fee_cents = obj.get("fee").and_then(|v| v.as_i64()).unwrap_or(0);

    // Fee accounting: gross revenue + separate fee expense
    // Debit: full amount to debit account
    // Credit: full amount to credit account (revenue at gross)
    // If fees: Debit Bank Fees, Credit debit account (net payout reduced)
    let mut lines = vec![
        TransactionLine {
            account_id: debit_id,
            debit: Money::from_cents(amount_cents),
            credit: Money::from_cents(0),
            memo: Some(format!("Stripe {}", event.event_type)),
        },
        TransactionLine {
            account_id: credit_id,
            debit: Money::from_cents(0),
            credit: Money::from_cents(amount_cents),
            memo: None,
        },
    ];

    // Add fee as separate balanced entries (debit expense, credit asset)
    if fee_cents > 0 && !is_refund {
        match aequi_storage::get_account_by_code(db, "5010").await? {
            Some(fee_account) => {
                let fee_id = fee_account.id.unwrap_or(aequi_core::AccountId(0));
                lines.push(TransactionLine {
                    account_id: fee_id,
                    debit: Money::from_cents(fee_cents),
                    credit: Money::from_cents(0),
                    memo: Some("Stripe processing fee".to_string()),
                });
                lines.push(TransactionLine {
                    account_id: debit_id,
                    debit: Money::from_cents(0),
                    credit: Money::from_cents(fee_cents),
                    memo: Some("Stripe fee offset".to_string()),
                });
            }
            None => {
                tracing::warn!("Bank Fees account (5010) not found, fee not recorded separately");
            }
        }
    }

    let validated = ValidatedTransaction::validate(UnvalidatedTransaction {
        date,
        description,
        lines,
        memo: Some(memo),
    })?;

    let (tx_id,): (i64,) = sqlx::query_as(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(validated.date.to_string())
    .bind(&validated.description)
    .bind(&validated.memo)
    .bind(validated.balanced_total.to_cents())
    .fetch_one(&mut *db_tx)
    .await?;

    for line in &validated.lines {
        sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(tx_id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *db_tx)
        .await?;
    }

    db_tx.commit().await?;

    tracing::info!(
        event_id = %event.id,
        transaction_id = tx_id,
        amount_cents,
        "Stripe webhook processed"
    );

    Ok(WebhookResponse {
        processed: true,
        event_id: event.id.clone(),
        transaction_id: Some(tx_id),
        message: "Transaction created".to_string(),
    })
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/stripe/webhook", post(handle_webhook))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_678_886_400;

    fn signed_header(secret: &str, t: i64, payload: &str) -> String {
        let sig = compute_signature(secret, &t.to_string(), payload.as_bytes());
        format!("t={t},v1={sig}")
    }

    #[test]
    fn payout_maps_to_checking() {
        let mapping = stripe_account_mapping("payout.paid");
        assert!(mapping.is_some());
        let (desc, debit, credit) = mapping.unwrap();
        assert_eq!(desc, "Stripe payout");
        assert_eq!(debit, "1000"); // Checking
        assert_eq!(credit, "1030"); // Undeposited Funds
    }

    #[test]
    fn charge_maps_to_revenue() {
        let mapping = stripe_account_mapping("charge.succeeded");
        assert!(mapping.is_some());
        let (_, debit, credit) = mapping.unwrap();
        assert_eq!(debit, "1030"); // Undeposited Funds
        assert_eq!(credit, "4000"); // Services Revenue
    }

    #[test]
    fn refund_reverses_revenue() {
        let mapping = stripe_account_mapping("charge.refunded");
        assert!(mapping.is_some());
        let (_, debit, credit) = mapping.unwrap();
        assert_eq!(debit, "4000"); // Revenue reversal
        assert_eq!(credit, "1030"); // Undeposited Funds
    }

    #[test]
    fn unknown_event_returns_none() {
        assert!(stripe_account_mapping("customer.created").is_none());
        assert!(stripe_account_mapping("invoice.payment_failed").is_none());
    }

    #[test]
    fn signature_verification_rejects_bad_sig() {
        let payload = b"test payload";
        let bad_sig = format!("t={NOW},v1=badhex");
        assert!(verify_stripe_signature(payload, &bad_sig, "whsec_test", NOW).is_err());
    }

    #[test]
    fn signature_verification_rejects_missing_timestamp() {
        assert!(verify_stripe_signature(b"test", "v1=abc", "secret", NOW).is_err());
    }

    #[test]
    fn signature_verification_rejects_missing_signature() {
        assert!(verify_stripe_signature(b"test", &format!("t={NOW}"), "secret", NOW).is_err());
    }

    #[test]
    fn valid_signature_accepted() {
        use sha2::{Digest, Sha256};

        let secret = "whsec_test_secret";
        let timestamp = "1678886400";
        let payload = r#"{"id":"evt_123","type":"charge.succeeded"}"#;

        let signed = format!("{timestamp}.{payload}");

        // Compute HMAC-SHA256 independently
        let key = secret.as_bytes();
        let block_size = 64;
        let mut ipad = vec![0x36u8; block_size];
        let mut opad = vec![0x5cu8; block_size];
        for (i, &b) in key.iter().enumerate() {
            ipad[i] ^= b;
            opad[i] ^= b;
        }
        let mut inner = Sha256::new();
        inner.update(&ipad);
        inner.update(signed.as_bytes());
        let inner_hash = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(&opad);
        outer.update(inner_hash);
        let expected = hex::encode(outer.finalize());

        let sig_header = format!("t={timestamp},v1={expected}");
        assert_eq!(
            verify_stripe_signature(payload.as_bytes(), &sig_header, secret, NOW),
            Ok(())
        );
    }

    #[test]
    fn stale_signature_rejected() {
        let payload = r#"{"id":"evt_1"}"#;
        let header = signed_header("s", NOW - 301, payload);
        assert!(verify_stripe_signature(payload.as_bytes(), &header, "s", NOW).is_err());
        let header = signed_header("s", NOW - 299, payload);
        assert!(verify_stripe_signature(payload.as_bytes(), &header, "s", NOW).is_ok());
    }

    #[test]
    fn replay_with_extra_timestamp_rejected() {
        // An old, validly signed header with a fresh `t` prepended used to pass:
        // the replay check read the first t and the HMAC the last.
        let payload = r#"{"id":"evt_old"}"#;
        let old = signed_header("s", NOW - 86_400, payload);
        let replay = format!("t={NOW},{old}");
        assert!(verify_stripe_signature(payload.as_bytes(), &replay, "s", NOW).is_err());
    }

    #[test]
    fn non_numeric_timestamp_rejected() {
        // A non-numeric t used to skip the replay check entirely.
        let payload = "{}";
        let sig = compute_signature("s", "abc", payload.as_bytes());
        let header = format!("t=abc,v1={sig}");
        assert_eq!(
            verify_stripe_signature(payload.as_bytes(), &header, "s", NOW),
            Err("non-numeric timestamp in signature header")
        );
    }

    #[test]
    fn any_of_multiple_v1_signatures_accepted() {
        let payload = r#"{"id":"evt_rot"}"#;
        let good = compute_signature("new", &NOW.to_string(), payload.as_bytes());
        let header = format!("t={NOW},v1={},v1={good},v0=legacy", "0".repeat(64));
        assert!(verify_stripe_signature(payload.as_bytes(), &header, "new", NOW).is_ok());
    }

    #[test]
    fn stripe_event_deserializes() {
        let json = r#"{
            "id": "evt_test_123",
            "type": "charge.succeeded",
            "created": 1678886400,
            "data": {
                "object": {
                    "id": "ch_abc",
                    "amount": 5000,
                    "fee": 175,
                    "currency": "usd"
                }
            }
        }"#;
        let event: StripeEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "charge.succeeded");
        assert_eq!(event.data.object["amount"], 5000);
        assert_eq!(event.data.object["fee"], 175);
    }

    fn event(id: &str, event_type: &str, object: serde_json::Value) -> StripeEvent {
        StripeEvent {
            id: id.into(),
            event_type: event_type.into(),
            data: StripeEventData { object },
            created: NOW,
        }
    }

    async fn revenue_balance(db: &aequi_storage::DbPool) -> i64 {
        let (cents,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(tl.credit_cents - tl.debit_cents), 0)
             FROM transaction_lines tl JOIN accounts a ON a.id = tl.account_id
             WHERE a.code = '4000'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        cents
    }

    #[tokio::test]
    async fn duplicate_event_is_booked_once() {
        let state = crate::routes::tests::test_state(None).await;
        let charge = event(
            "evt_1",
            "charge.succeeded",
            serde_json::json!({"id": "ch_1", "amount": 5000, "currency": "usd"}),
        );
        let first = process_event(&state.db, &charge).await.unwrap();
        assert!(first.processed);
        let retry = process_event(&state.db, &charge).await.unwrap();
        assert!(!retry.processed);
        assert_eq!(retry.transaction_id, first.transaction_id);
        assert_eq!(revenue_balance(&state.db).await, 5000);
    }

    #[tokio::test]
    async fn partial_refunds_book_only_the_new_amount() {
        let state = crate::routes::tests::test_state(None).await;
        let db = &state.db;
        let charge = serde_json::json!({"id": "ch_1", "amount": 10_000, "currency": "usd"});
        process_event(db, &event("evt_c", "charge.succeeded", charge))
            .await
            .unwrap();

        let refund = |evt: &str, refunded: i64| {
            event(
                evt,
                "charge.refunded",
                serde_json::json!({
                    "id": "ch_1", "amount": 10_000, "amount_refunded": refunded, "currency": "usd"
                }),
            )
        };
        assert!(
            process_event(db, &refund("evt_r1", 3_000))
                .await
                .unwrap()
                .processed
        );
        assert_eq!(revenue_balance(db).await, 7_000);
        assert!(
            process_event(db, &refund("evt_r2", 4_500))
                .await
                .unwrap()
                .processed
        );
        assert_eq!(revenue_balance(db).await, 5_500);
        // A later event that reports nothing new books nothing.
        let r3 = process_event(db, &refund("evt_r3", 4_500)).await.unwrap();
        assert!(!r3.processed);
        assert_eq!(revenue_balance(db).await, 5_500);
    }

    #[tokio::test]
    async fn non_usd_event_is_skipped() {
        let state = crate::routes::tests::test_state(None).await;
        let charge = event(
            "evt_jpy",
            "charge.succeeded",
            serde_json::json!({"id": "ch_j", "amount": 5000, "currency": "jpy"}),
        );
        let resp = process_event(&state.db, &charge).await.unwrap();
        assert!(!resp.processed);
        assert_eq!(revenue_balance(&state.db).await, 0);
    }
}
