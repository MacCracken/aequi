use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaidEnvironment {
    Sandbox,
    Development,
    Production,
}

impl PlaidEnvironment {
    pub fn base_url(&self) -> &str {
        match self {
            PlaidEnvironment::Sandbox => "https://sandbox.plaid.com",
            PlaidEnvironment::Development => "https://development.plaid.com",
            PlaidEnvironment::Production => "https://production.plaid.com",
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct PlaidConfig {
    pub client_id: String,
    pub secret: String,
    pub environment: PlaidEnvironment,
}

// Redact secret in Debug output
impl std::fmt::Debug for PlaidConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaidConfig")
            .field("client_id", &self.client_id)
            .field("secret", &"[REDACTED]")
            .field("environment", &self.environment)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PlaidError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Plaid API error: {error_type} — {error_message}")]
    Api {
        error_type: String,
        error_code: String,
        error_message: String,
    },

    #[error("Deserialization error: {0}")]
    Deserialize(String),
}

// ---------------------------------------------------------------------------
// API request/response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct LinkTokenCreateRequest {
    client_id: String,
    secret: String,
    user: LinkTokenUser,
    client_name: String,
    products: Vec<String>,
    country_codes: Vec<String>,
    language: String,
}

#[derive(Debug, Serialize)]
struct LinkTokenUser {
    client_user_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LinkTokenResponse {
    pub link_token: String,
    pub expiration: String,
    pub request_id: String,
}

#[derive(Debug, Serialize)]
struct ExchangeTokenRequest {
    client_id: String,
    secret: String,
    public_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExchangeTokenResponse {
    pub access_token: String,
    pub item_id: String,
    pub request_id: String,
}

#[derive(Debug, Serialize)]
struct TransactionsGetRequest {
    client_id: String,
    secret: String,
    access_token: String,
    start_date: String,
    end_date: String,
    options: TransactionsGetOptions,
}

#[derive(Debug, Serialize)]
struct TransactionsGetOptions {
    count: u32,
    offset: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TransactionsGetResponse {
    pub accounts: Vec<PlaidAccount>,
    pub transactions: Vec<PlaidTransaction>,
    pub total_transactions: u32,
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaidTransaction {
    pub transaction_id: String,
    pub name: String,
    pub amount: f64,
    pub date: String,
    pub category: Option<Vec<String>>,
    pub account_id: String,
    pub pending: bool,
    pub iso_currency_code: Option<String>,
    pub merchant_name: Option<String>,
}

impl PlaidTransaction {
    /// Convert the Plaid amount (f64 dollars) to cents (i64).
    /// Plaid amounts are positive for debits and negative for credits
    /// (from the user's perspective).
    pub fn amount_cents(&self) -> i64 {
        // Use string conversion to avoid f64 precision loss for large amounts
        let s = format!("{:.2}", self.amount);
        if let Some((whole, frac)) = s.split_once('.') {
            let sign = if self.amount < 0.0 { -1i64 } else { 1i64 };
            let whole_abs: i64 = whole.trim_start_matches('-').parse().unwrap_or(0);
            let frac_val: i64 = frac.parse().unwrap_or(0);
            sign * (whole_abs * 100 + frac_val)
        } else {
            (self.amount * 100.0).round() as i64
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaidAccount {
    pub account_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub subtype: Option<String>,
    pub balances: PlaidBalances,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaidBalances {
    pub available: Option<f64>,
    pub current: Option<f64>,
    pub limit: Option<f64>,
    pub iso_currency_code: Option<String>,
}

/// Plaid API error response body.
#[derive(Debug, Deserialize)]
struct PlaidApiErrorBody {
    error_type: String,
    error_code: String,
    error_message: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct PlaidClient {
    http: reqwest::Client,
    config: PlaidConfig,
}

impl PlaidClient {
    pub fn new(config: PlaidConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { http, config }
    }

    fn base_url(&self) -> &str {
        self.config.environment.base_url()
    }

    /// Create a Link token for initializing Plaid Link in the frontend.
    pub async fn create_link_token(&self, user_id: &str) -> Result<LinkTokenResponse, PlaidError> {
        let body = LinkTokenCreateRequest {
            client_id: self.config.client_id.clone(),
            secret: self.config.secret.clone(),
            user: LinkTokenUser {
                client_user_id: user_id.to_string(),
            },
            client_name: "Aequi".to_string(),
            products: vec!["transactions".to_string()],
            country_codes: vec!["US".to_string()],
            language: "en".to_string(),
        };

        let resp = self
            .http
            .post(format!("{}/link/token/create", self.base_url()))
            .json(&body)
            .send()
            .await?;

        self.handle_response(resp).await
    }

    /// Exchange a public token (received from Plaid Link) for an access token.
    pub async fn exchange_public_token(
        &self,
        public_token: &str,
    ) -> Result<ExchangeTokenResponse, PlaidError> {
        let body = ExchangeTokenRequest {
            client_id: self.config.client_id.clone(),
            secret: self.config.secret.clone(),
            public_token: public_token.to_string(),
        };

        let resp = self
            .http
            .post(format!("{}/item/public_token/exchange", self.base_url()))
            .json(&body)
            .send()
            .await?;

        self.handle_response(resp).await
    }

    /// Fetch all transactions for the given date range (handles pagination).
    pub async fn get_transactions(
        &self,
        access_token: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<TransactionsGetResponse, PlaidError> {
        let mut all_transactions = Vec::new();
        let mut accounts = Vec::new();
        let mut offset = 0u32;
        let page_size = 500u32;
        let mut total;
        let mut request_id;

        loop {
            let body = TransactionsGetRequest {
                client_id: self.config.client_id.clone(),
                secret: self.config.secret.clone(),
                access_token: access_token.to_string(),
                start_date: start_date.to_string(),
                end_date: end_date.to_string(),
                options: TransactionsGetOptions {
                    count: page_size,
                    offset,
                },
            };

            let resp = self
                .http
                .post(format!("{}/transactions/get", self.base_url()))
                .json(&body)
                .send()
                .await?;

            let page: TransactionsGetResponse = self.handle_response(resp).await?;

            total = page.total_transactions;
            request_id = page.request_id;
            if accounts.is_empty() {
                accounts = page.accounts;
            }
            all_transactions.extend(page.transactions);

            offset += page_size;
            if offset >= total {
                break;
            }
        }

        Ok(TransactionsGetResponse {
            accounts,
            transactions: all_transactions,
            total_transactions: total,
            request_id,
        })
    }

    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, PlaidError> {
        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            if let Ok(err_body) = serde_json::from_str::<PlaidApiErrorBody>(&text) {
                return Err(PlaidError::Api {
                    error_type: err_body.error_type,
                    error_code: err_body.error_code,
                    error_message: err_body.error_message,
                });
            }
            let mut truncated = text;
            truncated.truncate(500);
            return Err(PlaidError::Deserialize(format!(
                "HTTP {status}: {truncated}"
            )));
        }

        serde_json::from_str(&text).map_err(|e| PlaidError::Deserialize(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_deserializes() {
        let json = r#"{"client_id":"test_id","secret":"test_secret","environment":"sandbox"}"#;
        let parsed: PlaidConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.client_id, "test_id");
        assert_eq!(parsed.secret, "test_secret");
        assert_eq!(parsed.environment.base_url(), "https://sandbox.plaid.com");
    }

    #[test]
    fn config_debug_redacts_secret() {
        let config = PlaidConfig {
            client_id: "test_id".to_string(),
            secret: "super_secret_key".to_string(),
            environment: PlaidEnvironment::Sandbox,
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("super_secret_key"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn environment_base_urls() {
        assert_eq!(
            PlaidEnvironment::Sandbox.base_url(),
            "https://sandbox.plaid.com"
        );
        assert_eq!(
            PlaidEnvironment::Development.base_url(),
            "https://development.plaid.com"
        );
        assert_eq!(
            PlaidEnvironment::Production.base_url(),
            "https://production.plaid.com"
        );
    }

    #[test]
    fn plaid_transaction_deserializes() {
        let json = r#"{
            "transaction_id": "txn_abc123",
            "name": "Starbucks Coffee",
            "amount": 4.33,
            "date": "2026-03-10",
            "category": ["Food and Drink", "Coffee Shop"],
            "account_id": "acc_xyz",
            "pending": false,
            "iso_currency_code": "USD",
            "merchant_name": "Starbucks"
        }"#;
        let tx: PlaidTransaction = serde_json::from_str(json).unwrap();
        assert_eq!(tx.transaction_id, "txn_abc123");
        assert_eq!(tx.name, "Starbucks Coffee");
        assert_eq!(tx.amount_cents(), 433);
        assert!(!tx.pending);
        assert_eq!(tx.category.as_ref().unwrap().len(), 2);
        assert_eq!(tx.merchant_name.as_deref(), Some("Starbucks"));
    }

    #[test]
    fn plaid_transaction_negative_amount() {
        let json = r#"{
            "transaction_id": "txn_dep",
            "name": "Direct Deposit",
            "amount": -1500.50,
            "date": "2026-03-01",
            "category": null,
            "account_id": "acc_xyz",
            "pending": false,
            "iso_currency_code": "USD",
            "merchant_name": null
        }"#;
        let tx: PlaidTransaction = serde_json::from_str(json).unwrap();
        assert_eq!(tx.amount_cents(), -150050);
    }

    #[test]
    fn plaid_account_deserializes() {
        let json = r#"{
            "account_id": "acc_checking",
            "name": "My Checking",
            "type": "depository",
            "subtype": "checking",
            "balances": {
                "available": 1200.50,
                "current": 1300.00,
                "limit": null,
                "iso_currency_code": "USD"
            }
        }"#;
        let account: PlaidAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.account_id, "acc_checking");
        assert_eq!(account.account_type, "depository");
        assert_eq!(account.subtype.as_deref(), Some("checking"));
        assert_eq!(account.balances.available, Some(1200.50));
        assert!(account.balances.limit.is_none());
    }

    #[test]
    fn transactions_response_deserializes() {
        let json = r#"{
            "accounts": [{
                "account_id": "acc_1",
                "name": "Checking",
                "type": "depository",
                "subtype": "checking",
                "balances": { "available": 100.0, "current": 100.0, "limit": null, "iso_currency_code": "USD" }
            }],
            "transactions": [{
                "transaction_id": "txn_1",
                "name": "Coffee",
                "amount": 5.00,
                "date": "2026-03-10",
                "category": ["Food"],
                "account_id": "acc_1",
                "pending": false,
                "iso_currency_code": "USD",
                "merchant_name": null
            }],
            "total_transactions": 1,
            "request_id": "req_abc"
        }"#;
        let resp: TransactionsGetResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.accounts.len(), 1);
        assert_eq!(resp.transactions.len(), 1);
        assert_eq!(resp.total_transactions, 1);
        assert_eq!(resp.transactions[0].amount_cents(), 500);
    }

    #[test]
    fn link_token_response_deserializes() {
        let json = r#"{
            "link_token": "link-sandbox-abc123",
            "expiration": "2026-03-11T00:00:00Z",
            "request_id": "req_xyz"
        }"#;
        let resp: LinkTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.link_token, "link-sandbox-abc123");
    }

    #[test]
    fn exchange_token_response_deserializes() {
        let json = r#"{
            "access_token": "access-sandbox-token",
            "item_id": "item_abc",
            "request_id": "req_123"
        }"#;
        let resp: ExchangeTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "access-sandbox-token");
        assert_eq!(resp.item_id, "item_abc");
    }

    #[test]
    fn plaid_error_display() {
        let err = PlaidError::Api {
            error_type: "INVALID_REQUEST".to_string(),
            error_code: "MISSING_FIELDS".to_string(),
            error_message: "client_id is required".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("INVALID_REQUEST"));
        assert!(msg.contains("client_id is required"));
    }

    #[test]
    fn amount_cents_positive_integer() {
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Purchase".into(),
            amount: 42.0,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), 4200);
    }

    #[test]
    fn amount_cents_zero() {
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Zero".into(),
            amount: 0.0,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), 0);
    }

    #[test]
    fn amount_cents_small_fractional() {
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Penny".into(),
            amount: 0.01,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), 1);
    }

    #[test]
    fn amount_cents_large_amount() {
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Big purchase".into(),
            amount: 99999.99,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), 9999999);
    }

    #[test]
    fn amount_cents_negative_small() {
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Refund".into(),
            amount: -3.50,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), -350);
    }

    #[test]
    fn amount_cents_precision_edge() {
        // 19.99 can be tricky with f64
        let tx = PlaidTransaction {
            transaction_id: "t1".into(),
            name: "Item".into(),
            amount: 19.99,
            date: "2026-03-15".into(),
            category: None,
            account_id: "a1".into(),
            pending: false,
            iso_currency_code: None,
            merchant_name: None,
        };
        assert_eq!(tx.amount_cents(), 1999);
    }

    #[test]
    fn environment_serde_roundtrip() {
        for (env_str, expected_url) in [
            ("\"sandbox\"", "https://sandbox.plaid.com"),
            ("\"development\"", "https://development.plaid.com"),
            ("\"production\"", "https://production.plaid.com"),
        ] {
            let env: PlaidEnvironment = serde_json::from_str(env_str).unwrap();
            assert_eq!(env.base_url(), expected_url);
        }
    }

    #[test]
    fn config_all_environments() {
        for env_name in ["sandbox", "development", "production"] {
            let json = format!(
                r#"{{"client_id":"id","secret":"s","environment":"{}"}}"#,
                env_name
            );
            let config: PlaidConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(config.client_id, "id");
        }
    }

    #[test]
    fn plaid_client_base_url() {
        let config = PlaidConfig {
            client_id: "id".into(),
            secret: "secret".into(),
            environment: PlaidEnvironment::Sandbox,
        };
        let client = PlaidClient::new(config);
        assert_eq!(client.base_url(), "https://sandbox.plaid.com");

        let config_dev = PlaidConfig {
            client_id: "id".into(),
            secret: "secret".into(),
            environment: PlaidEnvironment::Development,
        };
        let client_dev = PlaidClient::new(config_dev);
        assert_eq!(client_dev.base_url(), "https://development.plaid.com");

        let config_prod = PlaidConfig {
            client_id: "id".into(),
            secret: "secret".into(),
            environment: PlaidEnvironment::Production,
        };
        let client_prod = PlaidClient::new(config_prod);
        assert_eq!(client_prod.base_url(), "https://production.plaid.com");
    }

    #[test]
    fn plaid_transaction_serde_roundtrip() {
        let tx = PlaidTransaction {
            transaction_id: "txn_abc".into(),
            name: "Amazon Purchase".into(),
            amount: 29.99,
            date: "2026-03-15".into(),
            category: Some(vec!["Shopping".into(), "Online".into()]),
            account_id: "acc_1".into(),
            pending: true,
            iso_currency_code: Some("USD".into()),
            merchant_name: Some("Amazon".into()),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let back: PlaidTransaction = serde_json::from_str(&json).unwrap();
        assert_eq!(back.transaction_id, "txn_abc");
        assert_eq!(back.name, "Amazon Purchase");
        assert_eq!(back.amount, 29.99);
        assert!(back.pending);
        assert_eq!(back.category.as_ref().unwrap().len(), 2);
        assert_eq!(back.merchant_name.as_deref(), Some("Amazon"));
    }

    #[test]
    fn plaid_account_serde_roundtrip() {
        let account = PlaidAccount {
            account_id: "acc_1".into(),
            name: "Checking".into(),
            account_type: "depository".into(),
            subtype: Some("checking".into()),
            balances: PlaidBalances {
                available: Some(5000.0),
                current: Some(5500.0),
                limit: None,
                iso_currency_code: Some("USD".into()),
            },
        };
        let json = serde_json::to_string(&account).unwrap();
        let back: PlaidAccount = serde_json::from_str(&json).unwrap();
        assert_eq!(back.account_id, "acc_1");
        assert_eq!(back.account_type, "depository");
        assert_eq!(back.balances.available, Some(5000.0));
    }

    #[test]
    fn plaid_api_error_body_deserializes() {
        let json = r#"{
            "error_type": "INVALID_REQUEST",
            "error_code": "MISSING_FIELDS",
            "error_message": "client_id is required"
        }"#;
        let err: PlaidApiErrorBody = serde_json::from_str(json).unwrap();
        assert_eq!(err.error_type, "INVALID_REQUEST");
        assert_eq!(err.error_code, "MISSING_FIELDS");
        assert_eq!(err.error_message, "client_id is required");
    }

    #[test]
    fn plaid_error_deserialize_variant() {
        let err = PlaidError::Deserialize("unexpected token".into());
        assert_eq!(err.to_string(), "Deserialization error: unexpected token");
    }

    #[test]
    fn plaid_transaction_no_optional_fields() {
        let json = r#"{
            "transaction_id": "txn_bare",
            "name": "Unknown",
            "amount": 10.00,
            "date": "2026-03-15",
            "category": null,
            "account_id": "acc_1",
            "pending": false,
            "iso_currency_code": null,
            "merchant_name": null
        }"#;
        let tx: PlaidTransaction = serde_json::from_str(json).unwrap();
        assert!(tx.category.is_none());
        assert!(tx.iso_currency_code.is_none());
        assert!(tx.merchant_name.is_none());
        assert_eq!(tx.amount_cents(), 1000);
    }

    #[test]
    fn plaid_balances_all_none() {
        let json = r#"{
            "available": null,
            "current": null,
            "limit": null,
            "iso_currency_code": null
        }"#;
        let balances: PlaidBalances = serde_json::from_str(json).unwrap();
        assert!(balances.available.is_none());
        assert!(balances.current.is_none());
        assert!(balances.limit.is_none());
        assert!(balances.iso_currency_code.is_none());
    }
}
