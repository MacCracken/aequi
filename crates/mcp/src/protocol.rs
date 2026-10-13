use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// A validated JSON-RPC 2.0 request or notification.
#[derive(Debug, Clone)]
pub struct JsonRpcRequest {
    /// `None` when the message had no `id` member: it is a notification and
    /// must not be answered. An explicit `"id": null` is still a request.
    pub id: Option<Value>,
    pub method: String,
    /// The `params` member, or `Value::Null` when absent.
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// Parse one JSON-RPC 2.0 message.
///
/// On failure, returns the error response to send back: a parse error for
/// malformed JSON, an invalid-request error for JSON that isn't a request
/// object. Either carries `"id": null` unless a valid id could be read.
pub fn parse_message(bytes: &[u8]) -> Result<JsonRpcRequest, JsonRpcResponse> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| {
        JsonRpcResponse::error(Value::Null, PARSE_ERROR, format!("Parse error: {e}"))
    })?;

    let Value::Object(mut obj) = value else {
        return Err(JsonRpcResponse::error(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: expected a single JSON-RPC request object".to_string(),
        ));
    };

    let id = match obj.remove("id") {
        None => None,
        Some(id @ (Value::Null | Value::String(_) | Value::Number(_))) => Some(id),
        Some(_) => {
            return Err(JsonRpcResponse::error(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: id must be a string, number or null".to_string(),
            ));
        }
    };
    let invalid = |message: &str| {
        JsonRpcResponse::error(
            id.clone().unwrap_or(Value::Null),
            INVALID_REQUEST,
            format!("Invalid Request: {message}"),
        )
    };

    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("jsonrpc must be \"2.0\""));
    }
    let method = match obj.remove("method") {
        Some(Value::String(m)) => m,
        _ => return Err(invalid("method must be a string")),
    };
    let params = match obj.remove("params") {
        None => Value::Null,
        Some(p @ (Value::Object(_) | Value::Array(_))) => p,
        Some(_) => return Err(invalid("params must be an object or array")),
    };

    Ok(JsonRpcRequest { id, method, params })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    /// Always serialized: `null` when the request id couldn't be determined.
    #[serde(default)]
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i64, message: String) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl ToolResult {
    pub fn text(text: String) -> Self {
        ToolResult {
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
            is_error: None,
        }
    }

    /// Pretty-printed JSON payload.
    pub fn json<T: Serialize>(value: &T) -> Self {
        match serde_json::to_string_pretty(value) {
            Ok(text) => ToolResult::text(text),
            Err(e) => ToolResult::error(format!("Failed to serialize result: {e}")),
        }
    }

    pub fn error(message: String) -> Self {
        ToolResult {
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text: message,
            }],
            is_error: Some(true),
        }
    }

    pub fn is_error(&self) -> bool {
        self.is_error.unwrap_or(false)
    }
}
