use serde::{Deserialize, Serialize};

/// Configuration for the AI categorization endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiCategorizationConfig {
    /// MCP server endpoint URL (e.g. "http://localhost:8061")
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

/// Suggest a category for a transaction by calling the configured MCP endpoint.
pub async fn suggest_category(
    config: &AiCategorizationConfig,
    description: &str,
    amount_cents: i64,
    date: &str,
    memo: Option<&str>,
    accounts: &[(String, String, String)], // (code, name, account_type)
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AiCategorizeError::RequestFailed(e.to_string()))?;

    let mut req = client
        .post(format!("{}/categorize", config.endpoint.trim_end_matches('/')))
        .json(&request);

    if let Some(ref key) = config.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AiCategorizeError::RequestFailed(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let mut body = resp
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        body.truncate(500);
        return Err(AiCategorizeError::RequestFailed(format!(
            "{status}: {body}"
        )));
    }

    let response: CategorizationResponse = resp
        .json()
        .await
        .map_err(|e| AiCategorizeError::InvalidResponse(e.to_string()))?;

    Ok(filter_by_confidence(response.suggestions, config.min_confidence))
}

/// Batch-suggest categories for multiple transactions.
pub async fn suggest_categories_batch(
    config: &AiCategorizationConfig,
    transactions: &[(String, i64, String, Option<String>)],
    accounts: &[(String, String, String)],
) -> Vec<Result<Vec<AiSuggestion>, AiCategorizeError>> {
    let mut results = Vec::with_capacity(transactions.len());
    for (desc, amount, date, memo) in transactions {
        let result = suggest_category(
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
}
