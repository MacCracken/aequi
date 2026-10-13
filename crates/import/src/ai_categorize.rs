use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::util::truncate_on_char_boundary;

/// Largest response body accepted from the categorization endpoint.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Longest model-provided reasoning kept, in bytes.
const MAX_REASONING_BYTES: usize = 1000;

/// Configuration for the AI categorization endpoint.
#[derive(Clone, Serialize, Deserialize)]
pub struct AiCategorizationConfig {
    /// MCP server endpoint URL (e.g. `http://localhost:8061`)
    pub endpoint: String,
    /// Optional Bearer token for authentication
    pub api_key: Option<String>,
    /// Confidence threshold below which suggestions are ignored (0.0-1.0)
    #[serde(default = "default_threshold")]
    pub min_confidence: f64,
}

fn default_threshold() -> f64 {
    0.7
}

// Redact the API key in Debug output
impl std::fmt::Debug for AiCategorizationConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiCategorizationConfig")
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("min_confidence", &self.min_confidence)
            .finish()
    }
}

impl AiCategorizationConfig {
    /// Clamp min_confidence to valid range [0.0, 1.0].
    pub fn validated(mut self) -> Self {
        self.min_confidence = self.min_confidence.clamp(0.0, 1.0);
        self
    }
}

/// A single categorization suggestion from the AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSuggestion {
    pub account_code: String,
    pub account_name: String,
    pub confidence: f64,
    pub reasoning: Option<String>,
}

/// Request sent to the MCP categorization endpoint.
#[derive(Debug, Clone, Serialize)]
struct CategorizationRequest {
    description: String,
    amount_cents: i64,
    date: String,
    memo: Option<String>,
    available_accounts: Vec<AccountOption>,
}

#[derive(Debug, Clone, Serialize)]
struct AccountOption {
    code: String,
    name: String,
    account_type: String,
}

/// Response from the MCP categorization endpoint.
#[derive(Debug, Deserialize)]
struct CategorizationResponse {
    suggestions: Vec<AiSuggestion>,
}

#[derive(Debug, thiserror::Error)]
pub enum AiCategorizeError {
    #[error("AI categorization request failed: {0}")]
    RequestFailed(String),
    #[error("AI categorization response invalid: {0}")]
    InvalidResponse(String),
}

/// Filter suggestions by confidence threshold.
pub fn filter_by_confidence(
    suggestions: Vec<AiSuggestion>,
    min_confidence: f64,
) -> Vec<AiSuggestion> {
    suggestions
        .into_iter()
        .filter(|s| s.confidence >= min_confidence)
        .collect()
}

/// Keep only suggestions that are usable as-is: the account code must be one
/// of the accounts offered (the model's account name is replaced by ours), the
/// confidence must be a number in `[0, 1]` and at least `min_confidence`, and
/// each account appears once. Results are ordered by confidence, highest first.
fn validate_suggestions(
    suggestions: Vec<AiSuggestion>,
    accounts: &[(String, String, String)],
    min_confidence: f64,
) -> Vec<AiSuggestion> {
    let mut valid: Vec<AiSuggestion> = Vec::new();
    for mut s in suggestions {
        if !(0.0..=1.0).contains(&s.confidence) {
            continue;
        }
        let code = s.account_code.trim();
        let Some((code, name, _)) = accounts.iter().find(|(c, _, _)| c == code) else {
            continue;
        };
        s.account_code = code.clone();
        s.account_name = name.clone();
        if let Some(reasoning) = s.reasoning.as_mut() {
            truncate_on_char_boundary(reasoning, MAX_REASONING_BYTES);
        }
        match valid.iter_mut().find(|v| v.account_code == s.account_code) {
            Some(existing) if existing.confidence >= s.confidence => {}
            Some(existing) => *existing = s,
            None => valid.push(s),
        }
    }
    valid.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    filter_by_confidence(valid, min_confidence)
}

fn http_client() -> Result<reqwest::Client, AiCategorizeError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AiCategorizeError::RequestFailed(e.without_url().to_string()))
}

/// Read a response body, refusing anything larger than `limit` bytes.
async fn read_body_limited(
    mut resp: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, AiCategorizeError> {
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| AiCategorizeError::RequestFailed(e.without_url().to_string()))?
    {
        if body.len() + chunk.len() > limit {
            return Err(AiCategorizeError::InvalidResponse(format!(
                "response larger than {limit} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Suggest a category for a transaction by calling the configured MCP endpoint.
pub async fn suggest_category(
    config: &AiCategorizationConfig,
    description: &str,
    amount_cents: i64,
    date: &str,
    memo: Option<&str>,
    accounts: &[(String, String, String)], // (code, name, account_type)
) -> Result<Vec<AiSuggestion>, AiCategorizeError> {
    let client = http_client()?;
    suggest_category_with(
        &client,
        config,
        description,
        amount_cents,
        date,
        memo,
        accounts,
    )
    .await
}

async fn suggest_category_with(
    client: &reqwest::Client,
    config: &AiCategorizationConfig,
    description: &str,
    amount_cents: i64,
    date: &str,
    memo: Option<&str>,
    accounts: &[(String, String, String)],
) -> Result<Vec<AiSuggestion>, AiCategorizeError> {
    let request = CategorizationRequest {
        description: description.to_string(),
        amount_cents,
        date: date.to_string(),
        memo: memo.map(String::from),
        available_accounts: accounts
            .iter()
            .map(|(code, name, at)| AccountOption {
                code: code.clone(),
                name: name.clone(),
                account_type: at.clone(),
            })
            .collect(),
    };

    let mut req = client
        .post(format!(
            "{}/categorize",
            config.endpoint.trim_end_matches('/')
        ))
        .json(&request);

    if let Some(ref key) = config.api_key {
        req = req.bearer_auth(key);
    }

    // Errors drop the URL: a configured endpoint may embed credentials.
    let resp = req
        .send()
        .await
        .map_err(|e| AiCategorizeError::RequestFailed(e.without_url().to_string()))?;

    let status = resp.status();
    let body = read_body_limited(resp, MAX_RESPONSE_BYTES).await;

    if !status.is_success() {
        let mut text = body
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_else(|_| "no body".to_string());
        truncate_on_char_boundary(&mut text, 500);
        return Err(AiCategorizeError::RequestFailed(format!(
            "{status}: {text}"
        )));
    }

    let response: CategorizationResponse = serde_json::from_slice(&body?)
        .map_err(|e| AiCategorizeError::InvalidResponse(e.to_string()))?;

    Ok(validate_suggestions(
        response.suggestions,
        accounts,
        config.min_confidence,
    ))
}

/// Batch-suggest categories for multiple transactions.
pub async fn suggest_categories_batch(
    config: &AiCategorizationConfig,
    transactions: &[(String, i64, String, Option<String>)],
    accounts: &[(String, String, String)],
) -> Vec<Result<Vec<AiSuggestion>, AiCategorizeError>> {
    let client = match http_client() {
        Ok(client) => client,
        Err(e) => {
            return transactions
                .iter()
                .map(|_| Err(AiCategorizeError::RequestFailed(e.to_string())))
                .collect();
        }
    };
    let mut results = Vec::with_capacity(transactions.len());
    for (desc, amount, date, memo) in transactions {
        let result = suggest_category_with(
            &client,
            config,
            desc,
            *amount,
            date,
            memo.as_deref(),
            accounts,
        )
        .await;
        results.push(result);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serde() {
        let json = r#"{
            "endpoint": "http://localhost:8061",
            "api_key": "test-key",
            "min_confidence": 0.8
        }"#;
        let config: AiCategorizationConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.endpoint, "http://localhost:8061");
        assert_eq!(config.min_confidence, 0.8);
    }

    #[test]
    fn config_default_threshold() {
        let json = r#"{"endpoint": "http://localhost:8061"}"#;
        let config: AiCategorizationConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.min_confidence, 0.7);
    }

    #[test]
    fn config_validated_clamps() {
        let config = AiCategorizationConfig {
            endpoint: "http://localhost".into(),
            api_key: None,
            min_confidence: 2.0,
        }
        .validated();
        assert_eq!(config.min_confidence, 1.0);

        let config2 = AiCategorizationConfig {
            endpoint: "http://localhost".into(),
            api_key: None,
            min_confidence: -0.5,
        }
        .validated();
        assert_eq!(config2.min_confidence, 0.0);
    }

    #[test]
    fn suggestion_serde() {
        let json = r#"{
            "account_code": "5020",
            "account_name": "Meals & Entertainment",
            "confidence": 0.92,
            "reasoning": "Contains 'STARBUCKS' which is a coffee shop"
        }"#;
        let s: AiSuggestion = serde_json::from_str(json).unwrap();
        assert_eq!(s.account_code, "5020");
        assert!(s.confidence > 0.9);
        assert!(s.reasoning.is_some());
    }

    #[test]
    fn suggestion_without_reasoning() {
        let json = r#"{
            "account_code": "5100",
            "account_name": "Office Supplies",
            "confidence": 0.75
        }"#;
        let s: AiSuggestion = serde_json::from_str(json).unwrap();
        assert!(s.reasoning.is_none());
    }

    #[test]
    fn filter_by_confidence_works() {
        let suggestions = vec![
            AiSuggestion {
                account_code: "5020".into(),
                account_name: "Meals".into(),
                confidence: 0.95,
                reasoning: None,
            },
            AiSuggestion {
                account_code: "5100".into(),
                account_name: "Office".into(),
                confidence: 0.5,
                reasoning: None,
            },
            AiSuggestion {
                account_code: "5110".into(),
                account_name: "Software".into(),
                confidence: 0.8,
                reasoning: None,
            },
        ];

        let filtered = filter_by_confidence(suggestions, 0.7);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].account_code, "5020");
        assert_eq!(filtered[1].account_code, "5110");
    }

    #[test]
    fn filter_by_confidence_all_pass() {
        let suggestions = vec![
            AiSuggestion {
                account_code: "5020".into(),
                account_name: "Meals".into(),
                confidence: 0.99,
                reasoning: None,
            },
            AiSuggestion {
                account_code: "5100".into(),
                account_name: "Office".into(),
                confidence: 0.85,
                reasoning: None,
            },
        ];
        let filtered = filter_by_confidence(suggestions, 0.5);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_by_confidence_none_pass() {
        let suggestions = vec![AiSuggestion {
            account_code: "5020".into(),
            account_name: "Meals".into(),
            confidence: 0.3,
            reasoning: None,
        }];
        let filtered = filter_by_confidence(suggestions, 0.5);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_by_confidence_empty_input() {
        let filtered = filter_by_confidence(vec![], 0.5);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_by_confidence_exact_threshold() {
        let suggestions = vec![AiSuggestion {
            account_code: "5020".into(),
            account_name: "Meals".into(),
            confidence: 0.7,
            reasoning: None,
        }];
        // Exactly at threshold should pass (>= comparison)
        let filtered = filter_by_confidence(suggestions, 0.7);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filter_by_confidence_zero_threshold() {
        let suggestions = vec![
            AiSuggestion {
                account_code: "5020".into(),
                account_name: "Meals".into(),
                confidence: 0.01,
                reasoning: None,
            },
            AiSuggestion {
                account_code: "5100".into(),
                account_name: "Office".into(),
                confidence: 0.0,
                reasoning: None,
            },
        ];
        let filtered = filter_by_confidence(suggestions, 0.0);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn config_validated_within_range() {
        let config = AiCategorizationConfig {
            endpoint: "http://localhost:8061".into(),
            api_key: Some("key".into()),
            min_confidence: 0.5,
        }
        .validated();
        assert_eq!(config.min_confidence, 0.5);
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = AiCategorizationConfig {
            endpoint: "http://localhost:8061".into(),
            api_key: Some("secret-key".into()),
            min_confidence: 0.85,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: AiCategorizationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint, "http://localhost:8061");
        assert_eq!(back.api_key.as_deref(), Some("secret-key"));
        assert_eq!(back.min_confidence, 0.85);
    }

    #[test]
    fn config_no_api_key() {
        let json = r#"{"endpoint": "http://localhost:8061"}"#;
        let config: AiCategorizationConfig = serde_json::from_str(json).unwrap();
        assert!(config.api_key.is_none());
        assert_eq!(config.min_confidence, 0.7); // default
    }

    #[test]
    fn suggestion_serde_roundtrip() {
        let s = AiSuggestion {
            account_code: "6010".into(),
            account_name: "Travel Expenses".into(),
            confidence: 0.88,
            reasoning: Some("Airline purchase detected".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AiSuggestion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.account_code, "6010");
        assert_eq!(back.account_name, "Travel Expenses");
        assert_eq!(back.confidence, 0.88);
        assert_eq!(back.reasoning.as_deref(), Some("Airline purchase detected"));
    }

    #[test]
    fn categorization_response_deserializes() {
        let json = r#"{
            "suggestions": [
                {
                    "account_code": "5020",
                    "account_name": "Meals",
                    "confidence": 0.92,
                    "reasoning": "Restaurant purchase"
                },
                {
                    "account_code": "5100",
                    "account_name": "Office",
                    "confidence": 0.45,
                    "reasoning": null
                }
            ]
        }"#;
        let resp: CategorizationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.suggestions.len(), 2);
        assert_eq!(resp.suggestions[0].account_code, "5020");
        assert_eq!(resp.suggestions[1].confidence, 0.45);
    }

    #[test]
    fn categorization_response_empty_suggestions() {
        let json = r#"{"suggestions": []}"#;
        let resp: CategorizationResponse = serde_json::from_str(json).unwrap();
        assert!(resp.suggestions.is_empty());
    }

    #[test]
    fn categorization_request_serializes() {
        let req = CategorizationRequest {
            description: "STARBUCKS #1234".into(),
            amount_cents: 525,
            date: "2026-03-15".into(),
            memo: Some("Coffee".into()),
            available_accounts: vec![AccountOption {
                code: "5020".into(),
                name: "Meals".into(),
                account_type: "Expense".into(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("STARBUCKS #1234"));
        assert!(json.contains("525"));
        assert!(json.contains("5020"));
    }

    #[test]
    fn ai_categorize_error_display() {
        let err = AiCategorizeError::RequestFailed("connection refused".into());
        assert_eq!(
            err.to_string(),
            "AI categorization request failed: connection refused"
        );
        let err2 = AiCategorizeError::InvalidResponse("missing field".into());
        assert_eq!(
            err2.to_string(),
            "AI categorization response invalid: missing field"
        );
    }

    fn accounts() -> Vec<(String, String, String)> {
        vec![
            ("5020".into(), "Meals".into(), "Expense".into()),
            ("5110".into(), "Software".into(), "Expense".into()),
        ]
    }

    fn suggestion(code: &str, name: &str, confidence: f64) -> AiSuggestion {
        AiSuggestion {
            account_code: code.into(),
            account_name: name.into(),
            confidence,
            reasoning: None,
        }
    }

    #[test]
    fn validate_drops_unknown_accounts_and_bad_confidence() {
        let suggestions = vec![
            suggestion("9999", "Made Up", 0.99),
            suggestion("5020", "Meals", 1.5),
            suggestion("5020", "Meals", f64::NAN),
            suggestion("5020", "Meals", -0.1),
            suggestion(" 5110 ", "Anything The Model Said", 0.8),
        ];
        let valid = validate_suggestions(suggestions, &accounts(), 0.7);
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].account_code, "5110");
        // Our account name, not the model's.
        assert_eq!(valid[0].account_name, "Software");
    }

    #[test]
    fn validate_dedupes_sorts_and_applies_threshold() {
        let suggestions = vec![
            suggestion("5110", "Software", 0.75),
            suggestion("5020", "Meals", 0.6),
            suggestion("5020", "Meals", 0.9),
            suggestion("5110", "Software", 0.71),
        ];
        let valid = validate_suggestions(suggestions, &accounts(), 0.7);
        let codes: Vec<_> = valid
            .iter()
            .map(|s| (s.account_code.as_str(), s.confidence))
            .collect();
        assert_eq!(codes, [("5020", 0.9), ("5110", 0.75)]);
    }

    #[test]
    fn validate_truncates_long_reasoning_on_char_boundary() {
        let mut s = suggestion("5020", "Meals", 0.9);
        s.reasoning = Some("é".repeat(2000));
        let valid = validate_suggestions(vec![s], &accounts(), 0.0);
        let reasoning = valid[0].reasoning.as_ref().unwrap();
        assert!(reasoning.len() <= MAX_REASONING_BYTES);
        assert!(reasoning.chars().all(|c| c == 'é'));
    }

    #[test]
    fn config_debug_redacts_api_key() {
        let config = AiCategorizationConfig {
            endpoint: "http://localhost:8061".into(),
            api_key: Some("super-secret-key".into()),
            min_confidence: 0.7,
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("super-secret-key"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn account_option_serializes() {
        let opt = AccountOption {
            code: "4010".into(),
            name: "Revenue".into(),
            account_type: "Income".into(),
        };
        let json = serde_json::to_string(&opt).unwrap();
        assert!(json.contains("4010"));
        assert!(json.contains("Revenue"));
        assert!(json.contains("Income"));
    }
}
