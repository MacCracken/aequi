//! Read-only Stripe API access. Webhook charge objects don't carry the
//! processing fee; it is on the charge's balance transaction, which only the
//! API returns.

use std::time::Duration;

use serde::Deserialize;

pub const DEFAULT_BASE_URL: &str = "https://api.stripe.com";

pub struct StripeApi {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

// Redact the key in Debug output
impl std::fmt::Debug for StripeApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StripeApi")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// The parts of a Stripe balance transaction the ledger needs.
#[derive(Debug, Clone, Deserialize)]
pub struct BalanceTransaction {
    pub id: String,
    /// Total Stripe fees, in the smallest unit of `currency`.
    pub fee: i64,
    pub currency: String,
    /// Unix time the balance transaction was created.
    pub created: i64,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
}

/// Accept only Stripe secret-side keys: a restricted key (`rk_…`, the
/// recommended choice) or a full secret key (`sk_…`). The message never
/// echoes the key.
fn validate_api_key(key: &str) -> Result<(), String> {
    let known_prefix = ["rk_live_", "rk_test_", "sk_live_", "sk_test_"]
        .iter()
        .any(|p| key.len() > p.len() && key.starts_with(p));
    if !known_prefix || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("expected a Stripe restricted key (rk_live_… or rk_test_…)".to_string());
    }
    Ok(())
}

impl StripeApi {
    pub fn new(api_key: &str) -> Result<Self, String> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// A client for `base_url` instead of api.stripe.com (a local test double).
    pub fn with_base_url(api_key: &str, base_url: &str) -> Result<Self, String> {
        validate_api_key(api_key)?;
        // Lookups run while Stripe waits for the webhook response.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Self {
            http,
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Restricted keys can be limited to reading balance transactions.
    pub fn is_restricted_key(&self) -> bool {
        self.api_key.starts_with("rk_")
    }

    /// `GET /v1/balance_transactions/{id}`. Errors describe the failure
    /// without the key or the response body.
    pub async fn balance_transaction(&self, id: &str) -> Result<BalanceTransaction, String> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err("invalid balance transaction id".to_string());
        }
        let resp = self
            .http
            .get(format!("{}/v1/balance_transactions/{id}", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| format!("Stripe request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp
                .json::<ErrorBody>()
                .await
                .ok()
                .and_then(|b| b.error.code.or(b.error.error_type))
                .map(|d| format!(" ({d})"))
                .unwrap_or_default();
            return Err(format!("Stripe returned HTTP {status}{detail}"));
        }
        resp.json::<BalanceTransaction>()
            .await
            .map_err(|e| format!("unexpected balance transaction response: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_restricted_and_secret_keys() {
        for key in ["rk_live_abc123", "rk_test_ABC", "sk_test_51abc"] {
            assert!(StripeApi::new(key).is_ok(), "{key}");
        }
        assert!(StripeApi::new("rk_test_x").unwrap().is_restricted_key());
        assert!(!StripeApi::new("sk_live_x").unwrap().is_restricted_key());
    }

    #[test]
    fn rejects_other_keys_without_echoing_them() {
        let expected = StripeApi::new("").unwrap_err();
        for key in [
            "pk_live_publishable",
            "whsec_signing_secret",
            "rk_test_",
            "rk_test_has space",
            "rk_test_x\n",
        ] {
            // The same message whatever the input, so nothing of it leaks.
            assert_eq!(StripeApi::new(key).unwrap_err(), expected, "{key:?}");
        }
    }

    #[test]
    fn debug_redacts_key() {
        let api = StripeApi::new("rk_test_supersecret").unwrap();
        let debug = format!("{api:?}");
        assert!(!debug.contains("supersecret"), "{debug}");
    }

    #[tokio::test]
    async fn rejects_ids_that_would_change_the_path() {
        let api = StripeApi::with_base_url("rk_test_k", "http://127.0.0.1:1").unwrap();
        for id in ["", "../v1/charges", "txn_1?expand=x", "txn 1"] {
            assert_eq!(
                api.balance_transaction(id).await.unwrap_err(),
                "invalid balance transaction id"
            );
        }
    }
}
