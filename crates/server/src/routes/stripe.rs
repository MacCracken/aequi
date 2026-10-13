use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

use crate::error::ApiError;
use crate::state::ServerState;
use crate::stripe_api::StripeApi;

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

/// Memo of the separate transaction that books a charge's processing fee
/// when the fee wasn't known yet as the charge was booked.
fn fee_memo(charge_id: &str) -> String {
    format!("Stripe fee: {charge_id}")
}

/// Expense account for Stripe processing fees (Bank Fees).
const FEE_ACCOUNT: &str = "5010";
/// Clearing account Stripe charges are booked to until paid out.
const CLEARING_ACCOUNT: &str = "1030";

/// Local calendar date of a unix timestamp.
fn local_date(unix_secs: i64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp(unix_secs, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// A charge's balance transaction id: a string, or an object with an `id`
/// when expanded. Null until Stripe creates it, which for asynchronous
/// payment methods is after `charge.succeeded`.
fn balance_transaction_id(charge: &serde_json::Value) -> Option<&str> {
    match charge.get("balance_transaction")? {
        serde_json::Value::String(id) => Some(id.as_str()),
        serde_json::Value::Object(bt) => bt.get("id")?.as_str(),
        _ => None,
    }
}

/// A processing fee read from a charge's balance transaction.
struct StripeFee {
    cents: i64,
    date: chrono::NaiveDate,
}

/// Look up the fee on balance transaction `bt_id`. `None` when there is
/// nothing to book: the lookup failed (logged; a later `charge.updated` can
/// still add the fee), the fee isn't in USD (logged), or there is no fee.
/// Never an error, so a failed lookup can't cost the charge's revenue.
async fn lookup_fee(
    api: &StripeApi,
    event: &StripeEvent,
    charge_id: &str,
    bt_id: &str,
) -> Option<StripeFee> {
    let bt = match api.balance_transaction(bt_id).await {
        Ok(bt) => bt,
        Err(error) => {
            tracing::warn!(
                event_id = %event.id,
                charge_id,
                %error,
                "Stripe fee lookup failed; the charge is booked without its fee"
            );
            return None;
        }
    };
    if !bt.currency.eq_ignore_ascii_case("usd") {
        tracing::warn!(
            event_id = %event.id,
            charge_id,
            balance_transaction = %bt.id,
            currency = %bt.currency,
            "Skipping non-USD Stripe fee; record it manually"
        );
        return None;
    }
    (bt.fee > 0).then(|| StripeFee {
        cents: bt.fee,
        date: local_date(bt.created),
    })
}

enum FeeStatus {
    /// No `charge.succeeded` booking for the charge.
    ChargeNotBooked,
    /// Booked with the charge or as a separate fee transaction.
    Booked,
    Missing,
}

async fn fee_status<'c>(
    executor: impl sqlx::SqliteExecutor<'c>,
    charge_id: &str,
    fee_account: aequi_core::AccountId,
) -> Result<FeeStatus, ApiError> {
    let charge_description = format!("Stripe charge: {charge_id}");
    let (charge_booked, fee_booked): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM transactions WHERE description = ?), \
                EXISTS(SELECT 1 FROM transactions WHERE memo = ?) \
                OR EXISTS(SELECT 1 FROM transaction_lines tl \
                          JOIN transactions t ON t.id = tl.transaction_id \
                          WHERE t.description = ? AND tl.account_id = ?)",
    )
    .bind(&charge_description)
    .bind(fee_memo(charge_id))
    .bind(&charge_description)
    .bind(fee_account.0)
    .fetch_one(executor)
    .await?;
    Ok(if fee_booked {
        FeeStatus::Booked
    } else if charge_booked {
        FeeStatus::Missing
    } else {
        FeeStatus::ChargeNotBooked
    })
}

/// `charge.updated` (only with `AEQUI_STRIPE_API_KEY`): book the processing
/// fee of a charge that was booked before Stripe created its balance
/// transaction, as a separate transaction. At most once per charge.
async fn book_late_fee(
    db: &aequi_storage::DbPool,
    api: &StripeApi,
    event: &StripeEvent,
) -> Result<WebhookResponse, ApiError> {
    let obj = &event.data.object;
    let Some(charge_id) = obj.get("id").and_then(|v| v.as_str()) else {
        return Ok(WebhookResponse::skipped(event, "Charge id missing"));
    };
    if obj.get("status").and_then(|v| v.as_str()) != Some("succeeded") {
        return Ok(WebhookResponse::skipped(event, "Charge has not succeeded"));
    }
    let Some(bt_id) = balance_transaction_id(obj) else {
        return Ok(WebhookResponse::skipped(
            event,
            "Charge has no balance transaction yet",
        ));
    };
    let Some(fee_account) = aequi_storage::get_account_by_code(db, FEE_ACCOUNT)
        .await?
        .and_then(|a| a.id)
    else {
        tracing::warn!("Bank Fees account ({FEE_ACCOUNT}) not found, fee not recorded");
        return Ok(WebhookResponse::skipped(event, "Fee account not found"));
    };
    let clearing_account = aequi_storage::get_account_by_code(db, CLEARING_ACCOUNT)
        .await?
        .and_then(|a| a.id)
        .ok_or_else(|| ApiError::Internal(format!("Account {CLEARING_ACCOUNT} not found")))?;

    // Most charge.updated events change nothing fee-related; settle those
    // from the ledger before spending an API call.
    match fee_status(db, charge_id, fee_account).await? {
        FeeStatus::ChargeNotBooked => {
            return Ok(WebhookResponse::skipped(
                event,
                "Charge not booked; fee not recorded",
            ));
        }
        FeeStatus::Booked => return Ok(WebhookResponse::skipped(event, "Fee already recorded")),
        FeeStatus::Missing => {}
    }
    let Some(fee) = lookup_fee(api, event, charge_id, bt_id).await else {
        return Ok(WebhookResponse::skipped(event, "No fee recorded"));
    };

    // Re-check under the write lock: a concurrent delivery may have booked it.
    let mut db_tx = db.begin_with("BEGIN IMMEDIATE").await?;
    if !matches!(
        fee_status(&mut *db_tx, charge_id, fee_account).await?,
        FeeStatus::Missing
    ) {
        return Ok(WebhookResponse::skipped(event, "Fee already recorded"));
    }
    let validated = ValidatedTransaction::validate(UnvalidatedTransaction {
        date: fee.date,
        description: format!("Stripe processing fee: {charge_id}"),
        lines: vec![
            TransactionLine {
                account_id: fee_account,
                debit: Money::from_cents(fee.cents),
                credit: Money::from_cents(0),
                memo: Some("Stripe processing fee".to_string()),
            },
            TransactionLine {
                account_id: clearing_account,
                debit: Money::from_cents(0),
                credit: Money::from_cents(fee.cents),
                memo: Some("Stripe fee offset".to_string()),
            },
        ],
        memo: Some(fee_memo(charge_id)),
    })?;
    let tx_id = aequi_storage::insert_ledger_transaction(&mut db_tx, &validated, None).await?;
    db_tx.commit().await?;

    tracing::info!(
        event_id = %event.id,
        charge_id,
        transaction_id = tx_id,
        fee_cents = fee.cents,
        "Stripe processing fee recorded"
    );
    Ok(WebhookResponse {
        processed: true,
        event_id: event.id.clone(),
        transaction_id: Some(tx_id),
        message: "Processing fee recorded".to_string(),
    })
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

    process_event(&state, &event).await.map(Json)
}

/// Book a verified Stripe event. Idempotent per event id, so Stripe's
/// retries (and duplicate deliveries) never post twice.
async fn process_event(
    state: &ServerState,
    event: &StripeEvent,
) -> Result<WebhookResponse, ApiError> {
    let db = &state.db;
    if event.event_type == "charge.updated"
        && let Some(api) = &state.stripe_api
    {
        return book_late_fee(db, api, event).await;
    }
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
    let date = local_date(event.created);

    let account_id = |code: &'static str| async move {
        aequi_storage::get_account_by_code(db, code)
            .await?
            .and_then(|a| a.id)
            .ok_or_else(|| ApiError::Internal(format!("Account {code} not found")))
    };
    let debit_id = account_id(debit_code).await?;
    let credit_id = account_id(credit_code).await?;

    // Webhook charges don't carry their fee; with an API key it is read from
    // the balance transaction. That happens before the write lock below is
    // taken. Without a balance transaction yet, the charge is booked gross
    // and charge.updated books the fee later.
    let looked_up_fee = match (&state.stripe_api, event.event_type.as_str()) {
        (Some(api), "charge.succeeded") if event_amount > 0 => match balance_transaction_id(obj) {
            Some(bt_id) => lookup_fee(api, event, stripe_id, bt_id).await,
            None => {
                tracing::info!(
                    event_id = %event.id,
                    charge_id = stripe_id,
                    "Charge has no balance transaction yet; its fee is booked on charge.updated"
                );
                None
            }
        },
        _ => None,
    };

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
    let fee_cents = match looked_up_fee {
        Some(fee) => fee.cents,
        None => obj.get("fee").and_then(|v| v.as_i64()).unwrap_or(0),
    };

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
        match aequi_storage::get_account_by_code(db, FEE_ACCOUNT).await? {
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
                tracing::warn!(
                    "Bank Fees account ({FEE_ACCOUNT}) not found, fee not recorded separately"
                );
            }
        }
    }

    let validated = ValidatedTransaction::validate(UnvalidatedTransaction {
        date,
        description,
        lines,
        memo: Some(memo),
    })?;

    // Same transaction as the duplicate check above, so a concurrent
    // delivery can't book the event twice.
    let tx_id = aequi_storage::insert_ledger_transaction(&mut db_tx, &validated, None).await?;

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
        let first = process_event(&state, &charge).await.unwrap();
        assert!(first.processed);
        let retry = process_event(&state, &charge).await.unwrap();
        assert!(!retry.processed);
        assert_eq!(retry.transaction_id, first.transaction_id);
        assert_eq!(revenue_balance(&state.db).await, 5000);
    }

    #[tokio::test]
    async fn partial_refunds_book_only_the_new_amount() {
        let state = crate::routes::tests::test_state(None).await;
        let db = &state.db;
        let charge = serde_json::json!({"id": "ch_1", "amount": 10_000, "currency": "usd"});
        process_event(&state, &event("evt_c", "charge.succeeded", charge))
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
            process_event(&state, &refund("evt_r1", 3_000))
                .await
                .unwrap()
                .processed
        );
        assert_eq!(revenue_balance(db).await, 7_000);
        assert!(
            process_event(&state, &refund("evt_r2", 4_500))
                .await
                .unwrap()
                .processed
        );
        assert_eq!(revenue_balance(db).await, 5_500);
        // A later event that reports nothing new books nothing.
        let r3 = process_event(&state, &refund("evt_r3", 4_500))
            .await
            .unwrap();
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
        let resp = process_event(&state, &charge).await.unwrap();
        assert!(!resp.processed);
        assert_eq!(revenue_balance(&state.db).await, 0);
    }

    async fn webhook(
        state: &Arc<ServerState>,
        payload: &str,
        signature: &str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        use tower::ServiceExt;

        let app = crate::routes::router(state.clone(), crate::routes::cors_layer(None).unwrap());
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/stripe/webhook")
            .header("content-type", "application/json")
            .header("stripe-signature", signature)
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn signed_charge_with_fee_is_booked_through_the_ledger() {
        use axum::http::StatusCode;

        let mut state = crate::routes::tests::test_state(None).await;
        Arc::get_mut(&mut state).unwrap().stripe_webhook_secret = Some("whsec_t".into());
        let now = chrono::Utc::now().timestamp();
        let payload = serde_json::json!({
            "id": "evt_fee",
            "type": "charge.succeeded",
            "created": now,
            "data": {"object": {"id": "ch_fee", "amount": 5000, "fee": 175, "currency": "usd"}}
        })
        .to_string();

        let (status, _) = webhook(&state, &payload, &signed_header("wrong", now, &payload)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let sig = signed_header("whsec_t", now, &payload);
        let (status, body) = webhook(&state, &payload, &sig).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["processed"], true);
        let tx_id = body["transaction_id"].as_i64().unwrap();

        let lines: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT a.code, tl.debit_cents, tl.credit_cents FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? ORDER BY tl.id",
        )
        .bind(tx_id)
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(
            lines,
            vec![
                ("1030".to_string(), 5000, 0),
                ("4000".to_string(), 0, 5000),
                ("5010".to_string(), 175, 0),
                ("1030".to_string(), 0, 175),
            ]
        );
        let (total, contact): (i64, Option<i64>) = sqlx::query_as(
            "SELECT balanced_total_cents, contact_id FROM transactions WHERE id = ?",
        )
        .bind(tx_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!((total, contact), (5175, None));

        // Stripe retries the delivery: not booked again.
        let (status, body) = webhook(&state, &payload, &sig).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["processed"], false);
        assert_eq!(body["transaction_id"], tx_id);
        assert_eq!(revenue_balance(&state.db).await, 5000);
    }

    #[tokio::test]
    async fn event_for_archived_account_is_rejected() {
        let state = crate::routes::tests::test_state(None).await;
        sqlx::query("UPDATE accounts SET is_archived = 1 WHERE code = '4000'")
            .execute(&state.db)
            .await
            .unwrap();
        let charge = event(
            "evt_arch",
            "charge.succeeded",
            serde_json::json!({"id": "ch_a", "amount": 5000, "currency": "usd"}),
        );
        let err = process_event(&state, &charge).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(ref m) if m.contains("archived")));
        assert_eq!(revenue_balance(&state.db).await, 0);
    }

    // ── Processing fees from the Stripe API ───────────────────────────────

    type Shared<T> = Arc<std::sync::Mutex<T>>;

    /// A stand-in for `GET /v1/balance_transactions/{id}` that records the
    /// id and Authorization header of each request.
    #[derive(Clone, Default)]
    struct MockStripe {
        /// Status and body by balance transaction id.
        responses: Shared<std::collections::HashMap<String, (u16, serde_json::Value)>>,
        /// (id, Authorization header) of each request.
        requests: Shared<Vec<(String, Option<String>)>>,
    }

    impl MockStripe {
        fn respond(&self, id: &str, status: u16, body: serde_json::Value) {
            self.responses
                .lock()
                .unwrap()
                .insert(id.to_string(), (status, body));
        }

        fn balance_transaction(&self, id: &str, fee: i64, currency: &str) {
            self.respond(
                id,
                200,
                serde_json::json!({
                    "id": id, "object": "balance_transaction", "amount": 10_000,
                    "fee": fee, "net": 10_000 - fee, "currency": currency, "created": NOW
                }),
            );
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }

        async fn start(&self) -> String {
            use axum::extract::Path;
            use axum::http::StatusCode;

            let mock = self.clone();
            let app = axum::Router::new().route(
                "/v1/balance_transactions/{id}",
                axum::routing::get(move |Path(id): Path<String>, headers: HeaderMap| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    mock.requests.lock().unwrap().push((id.clone(), auth));
                    let (status, body) = mock
                        .responses
                        .lock()
                        .unwrap()
                        .get(&id)
                        .cloned()
                        .unwrap_or((
                            404,
                            serde_json::json!({"error": {"type": "invalid_request_error", "code": "resource_missing"}}),
                        ));
                    (StatusCode::from_u16(status).unwrap(), Json(body))
                }),
            );
            crate::routes::tests::mock_server(app).await
        }
    }

    async fn state_with_stripe_api(mock: &MockStripe) -> Arc<ServerState> {
        let mut state = crate::routes::tests::test_state(None).await;
        Arc::get_mut(&mut state).unwrap().stripe_api =
            Some(StripeApi::with_base_url("rk_test_fees", &mock.start().await).unwrap());
        state
    }

    async fn ledger_lines(db: &aequi_storage::DbPool, tx_id: i64) -> Vec<(String, i64, i64)> {
        sqlx::query_as(
            "SELECT a.code, tl.debit_cents, tl.credit_cents FROM transaction_lines tl \
             JOIN accounts a ON a.id = tl.account_id WHERE tl.transaction_id = ? ORDER BY tl.id",
        )
        .bind(tx_id)
        .fetch_all(db)
        .await
        .unwrap()
    }

    async fn account_balance(db: &aequi_storage::DbPool, code: &str) -> i64 {
        let (cents,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(tl.debit_cents - tl.credit_cents), 0)
             FROM transaction_lines tl JOIN accounts a ON a.id = tl.account_id
             WHERE a.code = ?",
        )
        .bind(code)
        .fetch_one(db)
        .await
        .unwrap();
        cents
    }

    fn charge(id: &str, balance_transaction: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "id": id, "object": "charge", "amount": 10_000, "currency": "usd",
            "status": "succeeded", "balance_transaction": balance_transaction
        })
    }

    #[tokio::test]
    async fn charge_fee_is_read_from_its_balance_transaction() {
        let mock = MockStripe::default();
        mock.balance_transaction("txn_1", 320, "usd");
        let state = state_with_stripe_api(&mock).await;

        let resp = process_event(
            &state,
            &event("evt_1", "charge.succeeded", charge("ch_1", Some("txn_1"))),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(
            ledger_lines(&state.db, resp.transaction_id.unwrap()).await,
            vec![
                ("1030".to_string(), 10_000, 0),
                ("4000".to_string(), 0, 10_000),
                ("5010".to_string(), 320, 0),
                ("1030".to_string(), 0, 320),
            ]
        );
        // Net to the clearing account, gross revenue.
        assert_eq!(account_balance(&state.db, "1030").await, 9_680);
        assert_eq!(revenue_balance(&state.db).await, 10_000);
        assert_eq!(
            mock.requests.lock().unwrap().clone(),
            vec![("txn_1".to_string(), Some("Bearer rk_test_fees".to_string()))]
        );

        // A later charge.updated finds the fee booked without asking Stripe.
        let resp = process_event(
            &state,
            &event("evt_2", "charge.updated", charge("ch_1", Some("txn_1"))),
        )
        .await
        .unwrap();
        assert!(!resp.processed);
        assert_eq!(mock.request_count(), 1);
        assert_eq!(account_balance(&state.db, "5010").await, 320);
    }

    #[tokio::test]
    async fn async_charge_fee_is_booked_once_on_charge_updated() {
        let mock = MockStripe::default();
        mock.balance_transaction("txn_2", 250, "usd");
        let state = state_with_stripe_api(&mock).await;

        // No balance transaction yet: booked gross, no lookup.
        let resp = process_event(
            &state,
            &event("evt_s", "charge.succeeded", charge("ch_2", None)),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(
            ledger_lines(&state.db, resp.transaction_id.unwrap())
                .await
                .len(),
            2
        );
        assert_eq!(mock.request_count(), 0);

        let updated = |evt: &str| event(evt, "charge.updated", charge("ch_2", Some("txn_2")));
        let resp = process_event(&state, &updated("evt_u1")).await.unwrap();
        assert!(resp.processed, "{}", resp.message);
        let fee_tx = resp.transaction_id.unwrap();
        assert_eq!(
            ledger_lines(&state.db, fee_tx).await,
            vec![("5010".to_string(), 250, 0), ("1030".to_string(), 0, 250)]
        );
        let (memo,): (String,) = sqlx::query_as("SELECT memo FROM transactions WHERE id = ?")
            .bind(fee_tx)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(memo, "Stripe fee: ch_2");

        // Further updates (and retries) book nothing more.
        for evt in ["evt_u2", "evt_u1"] {
            let resp = process_event(&state, &updated(evt)).await.unwrap();
            assert!(!resp.processed);
        }
        assert_eq!(account_balance(&state.db, "5010").await, 250);
        assert_eq!(account_balance(&state.db, "1030").await, 9_750);
        assert_eq!(mock.request_count(), 1);
    }

    #[tokio::test]
    async fn failed_fee_lookup_still_books_the_charge() {
        let mock = MockStripe::default();
        mock.respond(
            "txn_3",
            401,
            serde_json::json!({"error": {
                "type": "invalid_request_error",
                "message": "Invalid API Key provided: rk_test_****fees"
            }}),
        );
        let state = state_with_stripe_api(&mock).await;

        let resp = process_event(
            &state,
            &event("evt_f", "charge.succeeded", charge("ch_3", Some("txn_3"))),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(revenue_balance(&state.db).await, 10_000);
        assert_eq!(account_balance(&state.db, "5010").await, 0);

        // Errors name the status, not the key or Stripe's message.
        let api = state.stripe_api.as_ref().unwrap();
        let err = api.balance_transaction("txn_3").await.unwrap_err();
        assert!(
            err.contains("401") && err.contains("invalid_request_error"),
            "{err}"
        );
        assert!(!err.contains("rk_test"), "{err}");

        // Once the lookup works, a later charge.updated adds the fee.
        mock.balance_transaction("txn_3", 330, "usd");
        let resp = process_event(
            &state,
            &event("evt_u", "charge.updated", charge("ch_3", Some("txn_3"))),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(account_balance(&state.db, "5010").await, 330);
    }

    #[tokio::test]
    async fn non_usd_fee_is_skipped() {
        let mock = MockStripe::default();
        mock.balance_transaction("txn_4", 300, "eur");
        let state = state_with_stripe_api(&mock).await;

        let resp = process_event(
            &state,
            &event("evt_e", "charge.succeeded", charge("ch_4", Some("txn_4"))),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(revenue_balance(&state.db).await, 10_000);
        assert_eq!(account_balance(&state.db, "5010").await, 0);

        let resp = process_event(
            &state,
            &event("evt_eu", "charge.updated", charge("ch_4", Some("txn_4"))),
        )
        .await
        .unwrap();
        assert!(!resp.processed);
        assert_eq!(account_balance(&state.db, "5010").await, 0);
    }

    #[tokio::test]
    async fn charge_updated_needs_a_booked_succeeded_charge() {
        let mock = MockStripe::default();
        mock.balance_transaction("txn_5", 300, "usd");
        let state = state_with_stripe_api(&mock).await;

        // Never booked (e.g. older than the webhook): no orphan fee.
        let resp = process_event(
            &state,
            &event("evt_a", "charge.updated", charge("ch_5", Some("txn_5"))),
        )
        .await
        .unwrap();
        assert!(!resp.processed);

        let mut failed = charge("ch_6", None);
        failed["status"] = "failed".into();
        let resp = process_event(&state, &event("evt_b", "charge.updated", failed))
            .await
            .unwrap();
        assert!(!resp.processed);
        assert_eq!(mock.request_count(), 0);
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn without_api_key_fees_are_not_looked_up() {
        let state = crate::routes::tests::test_state(None).await;
        let resp = process_event(
            &state,
            &event("evt_n", "charge.succeeded", charge("ch_7", Some("txn_7"))),
        )
        .await
        .unwrap();
        assert!(resp.processed);
        assert_eq!(
            ledger_lines(&state.db, resp.transaction_id.unwrap())
                .await
                .len(),
            2
        );

        let resp = process_event(
            &state,
            &event("evt_nu", "charge.updated", charge("ch_7", Some("txn_7"))),
        )
        .await
        .unwrap();
        assert!(!resp.processed);
        assert!(resp.message.contains("not mapped"), "{}", resp.message);
    }

    #[test]
    fn balance_transaction_id_accepts_id_or_expanded_object() {
        assert_eq!(
            balance_transaction_id(&charge("ch", Some("txn_1"))),
            Some("txn_1")
        );
        assert_eq!(balance_transaction_id(&charge("ch", None)), None);
        let expanded = serde_json::json!({"balance_transaction": {"id": "txn_2", "fee": 1}});
        assert_eq!(balance_transaction_id(&expanded), Some("txn_2"));
        assert_eq!(balance_transaction_id(&serde_json::json!({})), None);
    }
}
