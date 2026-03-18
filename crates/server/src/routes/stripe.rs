use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};

use aequi_core::{Money, TransactionLine, UnvalidatedTransaction, ValidatedTransaction};

use crate::state::ServerState;

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

// ── Signature verification ─────────────────────────────────────────────────

fn verify_stripe_signature(payload: &[u8], sig_header: &str, secret: &str) -> bool {
    use sha2::{Digest, Sha256};

    // Parse Stripe-Signature header: t=timestamp,v1=signature
    let mut timestamp = None;
    let mut signatures = Vec::new();

    for part in sig_header.split(',') {
        let part = part.trim();
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = Some(t.to_string());
        } else if let Some(v1) = part.strip_prefix("v1=") {
            signatures.push(v1.to_string());
        }
    }

    let timestamp = match timestamp {
        Some(t) => t,
        None => return false,
    };

    if signatures.is_empty() {
        return false;
    }

    // Compute expected signature: HMAC-SHA256(secret, "timestamp.payload")
    let signed_payload = format!("{timestamp}.{}", String::from_utf8_lossy(payload));

    // Use HMAC via manual SHA-256 (key XOR pad approach)
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
    inner_hasher.update(signed_payload.as_bytes());
    let inner_hash = inner_hasher.finalize();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&opad);
    outer_hasher.update(inner_hash);
    let expected = outer_hasher.finalize();

    let expected_hex = hex::encode(expected);

    // Constant-time comparison
    signatures.iter().any(|sig| {
        sig.len() == expected_hex.len()
            && sig
                .bytes()
                .zip(expected_hex.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    })
}

/// Extract the `t=` timestamp from a Stripe-Signature header.
fn parse_stripe_timestamp(sig_header: &str) -> Option<i64> {
    sig_header
        .split(',')
        .find_map(|part| part.trim().strip_prefix("t="))
        .and_then(|t| t.parse::<i64>().ok())
}

// ── Account mapping ────────────────────────────────────────────────────────

/// Map Stripe event types to debit/credit account codes.
fn stripe_account_mapping(event_type: &str) -> Option<(&str, &str, &str)> {
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

// ── Webhook handler ────────────────────────────────────────────────────────

async fn handle_webhook(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // SECURITY: Always require webhook secret — reject if not configured
    let secret = match state.stripe_webhook_secret.as_ref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({ "error": "Stripe webhook not configured" })),
            );
        }
    };

    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_stripe_signature(&body, sig, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "Invalid signature" })),
        );
    }

    // Replay protection: reject signatures older than 300 seconds
    if let Some(timestamp) = parse_stripe_timestamp(sig) {
        let now = chrono::Utc::now().timestamp();
        if (now - timestamp).unsigned_abs() > 300 {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "Signature timestamp too old" })),
            );
        }
    }

    let event: StripeEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": format!("Invalid event: {e}") })),
            );
        }
    };

    tracing::info!(
        event_id = %event.id,
        event_type = %event.event_type,
        "Processing Stripe webhook"
    );

    let mapping = match stripe_account_mapping(&event.event_type) {
        Some(m) => m,
        None => {
            return (
                StatusCode::OK,
                axum::Json(serde_json::json!(WebhookResponse {
                    processed: false,
                    event_id: event.id,
                    transaction_id: None,
                    message: format!("Event type '{}' not mapped", event.event_type),
                })),
            );
        }
    };

    let (desc_prefix, debit_code, credit_code) = mapping;
    let obj = &event.data.object;

    // Extract amount (Stripe uses cents)
    let amount_cents = obj.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);

    if amount_cents == 0 {
        return (
            StatusCode::OK,
            axum::Json(serde_json::json!(WebhookResponse {
                processed: false,
                event_id: event.id,
                transaction_id: None,
                message: "Zero amount, skipped".to_string(),
            })),
        );
    }

    // Extract date from event timestamp
    let date = chrono::DateTime::from_timestamp(event.created, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive());

    // Extract description details
    let stripe_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or(&event.id);
    let description = format!("{desc_prefix}: {stripe_id}");

    // Look up accounts
    let debit_account = match aequi_storage::get_account_by_code(&state.db, debit_code).await {
        Ok(Some(a)) => a,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({ "error": format!("Debit account {debit_code} not found") }),
                ),
            );
        }
    };

    let credit_account = match aequi_storage::get_account_by_code(&state.db, credit_code).await {
        Ok(Some(a)) => a,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({ "error": format!("Credit account {credit_code} not found") }),
                ),
            );
        }
    };

    // Handle Stripe fees as a separate line if present
    let fee_cents = obj.get("fee").and_then(|v| v.as_i64()).unwrap_or(0);

    // Guard against negative amounts
    if amount_cents < 0 {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "Negative amount" })),
        );
    }

    let debit_id = debit_account.id.unwrap_or(aequi_core::AccountId(0));
    let credit_id = credit_account.id.unwrap_or(aequi_core::AccountId(0));

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
    if fee_cents > 0 {
        match aequi_storage::get_account_by_code(&state.db, "5010").await {
            Ok(Some(fee_account)) => {
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
            _ => {
                tracing::warn!("Bank Fees account (5010) not found, fee not recorded separately");
            }
        }
    }

    let tx = UnvalidatedTransaction {
        date,
        description,
        lines,
        memo: Some(format!("Stripe event: {}", event.id)),
    };

    let validated = match ValidatedTransaction::validate(tx) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({ "error": format!("Transaction validation failed: {e}") }),
                ),
            );
        }
    };

    // Insert transaction atomically
    let mut db_tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({ "error": format!("DB transaction begin failed: {e}") }),
                ),
            );
        }
    };

    let row = match sqlx::query_as::<_, (i64,)>(
        "INSERT INTO transactions (date, description, memo, balanced_total_cents) VALUES (?, ?, ?, ?) RETURNING id"
    )
    .bind(validated.date.to_string())
    .bind(&validated.description)
    .bind(&validated.memo)
    .bind(validated.balanced_total.to_cents())
    .fetch_one(&mut *db_tx)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = db_tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": format!("Failed to insert transaction: {e}") })),
            );
        }
    };

    let tx_id = row.0;

    for line in &validated.lines {
        if let Err(e) = sqlx::query(
            "INSERT INTO transaction_lines (transaction_id, account_id, debit_cents, credit_cents, memo) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(tx_id)
        .bind(line.account_id.0)
        .bind(line.debit.to_cents())
        .bind(line.credit.to_cents())
        .bind(&line.memo)
        .execute(&mut *db_tx)
        .await
        {
            let _ = db_tx.rollback().await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": format!("Failed to insert line: {e}") })),
            );
        }
    }

    if let Err(e) = db_tx.commit().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": format!("DB commit failed: {e}") })),
        );
    }

    tracing::info!(
        event_id = %event.id,
        transaction_id = tx_id,
        amount_cents,
        "Stripe webhook processed"
    );

    (
        StatusCode::OK,
        axum::Json(serde_json::json!(WebhookResponse {
            processed: true,
            event_id: event.id,
            transaction_id: Some(tx_id),
            message: "Transaction created".to_string(),
        })),
    )
}

pub fn routes() -> Router<Arc<ServerState>> {
    Router::new().route("/stripe/webhook", post(handle_webhook))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let bad_sig = "t=1234567890,v1=badhex";
        assert!(!verify_stripe_signature(payload, bad_sig, "whsec_test"));
    }

    #[test]
    fn signature_verification_rejects_missing_timestamp() {
        assert!(!verify_stripe_signature(b"test", "v1=abc", "secret"));
    }

    #[test]
    fn signature_verification_rejects_missing_signature() {
        assert!(!verify_stripe_signature(b"test", "t=123", "secret"));
    }

    #[test]
    fn valid_signature_accepted() {
        use sha2::{Digest, Sha256};

        let secret = "whsec_test_secret";
        let timestamp = "1678886400";
        let payload = r#"{"id":"evt_123","type":"charge.succeeded"}"#;

        let signed = format!("{timestamp}.{payload}");

        // Compute HMAC-SHA256 manually
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
        assert!(verify_stripe_signature(
            payload.as_bytes(),
            &sig_header,
            secret
        ));
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
}
