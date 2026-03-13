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

/// A single categorization suggestion from the AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSuggestion {
    pub account_code: String,
    pub account_name: String,
    pub confidence: f64,
    pub reasoning: Option<String>,
}

/// Request sent to the MCP categorization endpoint.
#[derive(Debug, Serialize)]
struct CategorizationRequest {
    description: String,
    amount_cents: i64,
    date: String,
    memo: Option<String>,
    available_accounts: Vec<AccountOption>,
}

#[derive(Debug, Serialize)]
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

/// Suggest a category for a transaction by calling the configured MCP endpoint.
///
/// The endpoint should accept a JSON body with transaction details and available
/// accounts, and return a list of suggestions ranked by confidence.
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

    let client = reqwest::Client::new();
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
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        return Err(AiCategorizeError::RequestFailed(format!(
            "{status}: {body}"
        )));
    }

    let response: CategorizationResponse = resp
        .json()
        .await
        .map_err(|e| AiCategorizeError::InvalidResponse(e.to_string()))?;

    // Filter by confidence threshold
    let suggestions = response
        .suggestions
        .into_iter()
        .filter(|s| s.confidence >= config.min_confidence)
        .collect();

    Ok(suggestions)
}

/// Batch-suggest categories for multiple transactions.
pub async fn suggest_categories_batch(
    config: &AiCategorizationConfig,
    transactions: &[(String, i64, String, Option<String>)], // (description, amount_cents, date, memo)
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
}
