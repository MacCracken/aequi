//! MCP HTTP+SSE transport (MCP specification 2024-11-05).
//!
//! The client:
//! 1. Opens `GET /sse` to receive server-sent events (session endpoint URL).
//! 2. Sends JSON-RPC requests via `POST /message?sessionId=<id>`.
//! 3. Receives JSON-RPC responses as SSE `message` events on the `/sse` stream.

#[cfg(feature = "sse")]
pub mod transport {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use tokio::sync::{mpsc, RwLock};
    use tower_http::cors::CorsLayer;

    use crate::audit;
    use crate::permissions::Permissions;
    use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
    use crate::tools::ToolRegistry;

    /// Shared state for the SSE MCP server.
    pub struct SseState {
        pub db: aequi_storage::DbPool,
        pub permissions: Permissions,
        pub registry: ToolRegistry,
        /// Active SSE sessions: session_id → sender channel.
        pub sessions: RwLock<HashMap<String, mpsc::Sender<JsonRpcResponse>>>,
    }

    /// Build the Axum router for the MCP SSE transport.
    pub fn router(state: Arc<SseState>) -> Router {
        Router::new()
            .route("/sse", get(sse_handler))
            .route("/message", post(message_handler))
            .layer(CorsLayer::permissive())
            .with_state(state)
    }

    /// `GET /sse` — opens an SSE stream for a new session.
    ///
    /// Sends an initial `endpoint` event with the message URL, then streams
    /// JSON-RPC responses as `message` events.
    async fn sse_handler(
        State(state): State<Arc<SseState>>,
    ) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel::<JsonRpcResponse>(64);

        state
            .sessions
            .write()
            .await
            .insert(session_id.clone(), tx);

        let endpoint_url = format!("/message?sessionId={session_id}");

        let stream = async_stream::stream! {
            // First event: tell the client where to POST messages
            yield Ok(Event::default()
                .event("endpoint")
                .data(endpoint_url));

            // Subsequent events: JSON-RPC responses
            let mut rx = rx;
            while let Some(response) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&response) {
                    yield Ok(Event::default()
                        .event("message")
                        .data(json));
                }
            }
        };

        Sse::new(stream).keep_alive(KeepAlive::default())
    }

    #[derive(serde::Deserialize)]
    struct MessageQuery {
        #[serde(rename = "sessionId")]
        session_id: String,
    }

    /// `POST /message?sessionId=<id>` — receives a JSON-RPC request and pushes
    /// the response onto the session's SSE stream.
    async fn message_handler(
        State(state): State<Arc<SseState>>,
        Query(query): Query<MessageQuery>,
        Json(request): Json<JsonRpcRequest>,
    ) -> impl IntoResponse {
        let sessions = state.sessions.read().await;
        let tx = match sessions.get(&query.session_id) {
            Some(tx) => tx.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(JsonRpcResponse::error(
                        request.id,
                        -32000,
                        "Unknown session".to_string(),
                    )),
                );
            }
        };
        drop(sessions);

        let response =
            handle_request(&request, &state.registry, &state.db, &state.permissions).await;

        // Push response to SSE stream (best-effort)
        let _ = tx.send(response.clone()).await;

        (StatusCode::ACCEPTED, Json(response))
    }

    /// Same request handler as stdio transport, extracted for reuse.
    async fn handle_request(
        req: &JsonRpcRequest,
        registry: &ToolRegistry,
        db: &aequi_storage::DbPool,
        permissions: &Permissions,
    ) -> JsonRpcResponse {
        use serde_json::json;

        match req.method.as_str() {
            "initialize" => JsonRpcResponse::success(
                req.id.clone(),
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false },
                    },
                    "serverInfo": {
                        "name": "aequi-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "notifications/initialized" => JsonRpcResponse::success(req.id.clone(), json!({})),
            "tools/list" => {
                let tools: Vec<_> = registry
                    .list_definitions()
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema
                        })
                    })
                    .collect();
                JsonRpcResponse::success(req.id.clone(), json!({ "tools": tools }))
            }
            "tools/call" => {
                let tool_name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

                let input_str = serde_json::to_string(&arguments).unwrap_or_default();
                let result = registry.call(tool_name, arguments, db, permissions).await;

                let outcome = if result.is_error.unwrap_or(false) {
                    "error"
                } else {
                    "success"
                };
                audit::log_tool_call(db, tool_name, &input_str, outcome).await;

                JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(&result).unwrap_or(json!({})),
                )
            }
            _ => JsonRpcResponse::error(
                req.id.clone(),
                -32601,
                format!("Method not found: {}", req.method),
            ),
        }
    }

    /// Start the MCP SSE server on the given port.
    pub async fn run_sse_server(
        db: aequi_storage::DbPool,
        permissions: Permissions,
        port: u16,
    ) -> anyhow::Result<()> {
        let state = Arc::new(SseState {
            db,
            permissions,
            registry: ToolRegistry::new(),
            sessions: RwLock::new(HashMap::new()),
        });

        let app = router(state);
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
        tracing::info!("aequi-mcp SSE server listening on port {port}");
        axum::serve(listener, app).await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;
        use axum::body::Body;
        use std::path::PathBuf;
        use tower::ServiceExt;

        fn req(method: &str, uri: &str, body: Body) -> axum::http::Request<Body> {
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body)
                .unwrap()
        }

        async fn test_state() -> Arc<SseState> {
            let db = aequi_storage::create_db(&PathBuf::from(":memory:"))
                .await
                .unwrap();
            aequi_storage::seed_default_accounts(&db).await.unwrap();
            Arc::new(SseState {
                db,
                permissions: Permissions::default(),
                registry: ToolRegistry::new(),
                sessions: RwLock::new(HashMap::new()),
            })
        }

        async fn setup_session(state: &Arc<SseState>, id: &str) {
            let (tx, _rx) = mpsc::channel(64);
            state.sessions.write().await.insert(id.to_string(), tx);
        }

        fn json_body(value: &serde_json::Value) -> Body {
            Body::from(serde_json::to_string(value).unwrap())
        }

        #[tokio::test]
        async fn sse_endpoint_returns_event_stream() {
            let state = test_state().await;
            let app = router(state);

            let response = app
                .oneshot(req("GET", "/sse", Body::empty()))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let ct = response.headers().get("content-type").unwrap().to_str().unwrap();
            assert!(ct.contains("text/event-stream"));
        }

        #[tokio::test]
        async fn message_unknown_session_returns_404() {
            let state = test_state().await;
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "initialize", "params": {}
            });

            let response = app
                .oneshot(req("POST", "/message?sessionId=nonexistent", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn message_with_valid_session() {
            let state = test_state().await;
            setup_session(&state, "s1").await;
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "initialize", "params": {}
            });

            let response = app
                .oneshot(req("POST", "/message?sessionId=s1", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        #[tokio::test]
        async fn message_tools_list() {
            let state = test_state().await;
            setup_session(&state, "s2").await;
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 2,
                "method": "tools/list", "params": {}
            });

            let response = app
                .oneshot(req("POST", "/message?sessionId=s2", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let resp: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
            let tools = resp.result.unwrap();
            assert_eq!(tools["tools"].as_array().unwrap().len(), 24);
        }

        #[tokio::test]
        async fn message_tool_call() {
            let state = test_state().await;
            setup_session(&state, "s3").await;
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 3,
                "method": "tools/call",
                "params": { "name": "aequi_list_accounts", "arguments": {} }
            });

            let response = app
                .oneshot(req("POST", "/message?sessionId=s3", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let resp: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
            let result = resp.result.unwrap();
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Checking"));
        }

        #[tokio::test]
        async fn message_unknown_method() {
            let state = test_state().await;
            setup_session(&state, "s4").await;
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 4,
                "method": "bogus/method", "params": {}
            });

            let response = app
                .oneshot(req("POST", "/message?sessionId=s4", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let resp: JsonRpcResponse = serde_json::from_slice(&bytes).unwrap();
            assert!(resp.error.is_some());
            assert_eq!(resp.error.unwrap().code, -32601);
        }

        #[tokio::test]
        async fn session_cleanup() {
            let state = test_state().await;
            let session_id = "cleanup-test".to_string();
            let (tx, rx) = mpsc::channel(64);
            state
                .sessions
                .write()
                .await
                .insert(session_id.clone(), tx);

            assert_eq!(state.sessions.read().await.len(), 1);

            // Drop the receiver to simulate client disconnect
            drop(rx);

            // The sender is still in the map, but sends will fail
            let sessions = state.sessions.read().await;
            let tx = sessions.get(&session_id).unwrap();
            let resp = JsonRpcResponse::success(None, serde_json::json!({}));
            assert!(tx.send(resp).await.is_err());
        }
    }
}
