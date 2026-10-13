//! MCP HTTP+SSE transport (MCP specification 2024-11-05).
//!
//! The client:
//! 1. Opens `GET /sse` to receive server-sent events (session endpoint URL).
//! 2. Sends JSON-RPC requests via `POST /message?sessionId=<id>`.
//! 3. Receives JSON-RPC responses as SSE `message` events on the `/sse` stream.
//!
//! Both endpoints require `Authorization: Bearer <AEQUI_MCP_TOKEN>`. The server
//! binds to 127.0.0.1 unless `AEQUI_MCP_BIND` says otherwise.

#[cfg(feature = "sse")]
pub mod transport {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use anyhow::{Context, bail};
    use axum::body::Bytes;
    use axum::extract::{Query, Request, State};
    use axum::http::{StatusCode, header};
    use axum::middleware::{self, Next};
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tokio::sync::mpsc;

    use crate::permissions::Permissions;
    use crate::protocol::JsonRpcResponse;
    use crate::server::McpServer;

    pub const DEFAULT_PORT: u16 = 8061;
    /// Concurrent `/sse` streams allowed; further connections get 503.
    pub const MAX_SESSIONS: usize = 16;

    /// Listener settings, read from the environment.
    #[derive(Debug, Clone)]
    pub struct SseConfig {
        pub addr: SocketAddr,
        pub token: String,
    }

    impl SseConfig {
        /// `AEQUI_MCP_BIND` (default 127.0.0.1), `AEQUI_MCP_PORT` (default
        /// 8061) and the required `AEQUI_MCP_TOKEN`.
        pub fn from_env() -> anyhow::Result<Self> {
            Self::from_vars(
                std::env::var("AEQUI_MCP_BIND").ok(),
                std::env::var("AEQUI_MCP_PORT").ok(),
                std::env::var("AEQUI_MCP_TOKEN").ok(),
            )
        }

        pub fn from_vars(
            bind: Option<String>,
            port: Option<String>,
            token: Option<String>,
        ) -> anyhow::Result<Self> {
            let token = token.unwrap_or_default();
            if token.trim().is_empty() {
                bail!(
                    "AEQUI_MCP_TOKEN must be set to a non-empty secret to use the SSE transport; \
                     clients must send it as `Authorization: Bearer <token>`"
                );
            }
            let ip: IpAddr = match bind.as_deref().map(str::trim) {
                None | Some("") => IpAddr::V4(Ipv4Addr::LOCALHOST),
                Some(b) => b
                    .parse()
                    .with_context(|| format!("AEQUI_MCP_BIND is not an IP address: {b:?}"))?,
            };
            let port: u16 = match port.as_deref().map(str::trim) {
                None | Some("") => DEFAULT_PORT,
                Some(p) => p
                    .parse()
                    .with_context(|| format!("AEQUI_MCP_PORT is not a valid port: {p:?}"))?,
            };
            Ok(SseConfig {
                addr: SocketAddr::new(ip, port),
                token,
            })
        }
    }

    /// Shared state for the SSE MCP server.
    pub struct SseState {
        pub server: McpServer,
        token_digest: [u8; 32],
        /// Active SSE sessions: session_id → sender channel. Entries are
        /// removed when the client's stream is dropped.
        sessions: Mutex<HashMap<String, mpsc::Sender<JsonRpcResponse>>>,
    }

    impl SseState {
        pub fn new(server: McpServer, token: &str) -> anyhow::Result<Self> {
            if token.trim().is_empty() {
                bail!("the SSE transport requires a non-empty bearer token");
            }
            Ok(SseState {
                server,
                token_digest: Sha256::digest(token.as_bytes()).into(),
                sessions: Mutex::new(HashMap::new()),
            })
        }

        fn sessions(&self) -> MutexGuard<'_, HashMap<String, mpsc::Sender<JsonRpcResponse>>> {
            self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub fn session_count(&self) -> usize {
            self.sessions().len()
        }

        /// Comparing SHA-256 digests keeps the check independent of the
        /// token's length, and the fold avoids an early exit.
        fn token_matches(&self, presented: &str) -> bool {
            let digest: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
            digest
                .iter()
                .zip(self.token_digest.iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
    }

    /// Removes the session from the map when its SSE stream is dropped.
    struct SessionGuard {
        state: Arc<SseState>,
        id: String,
    }

    impl Drop for SessionGuard {
        fn drop(&mut self) {
            self.state.sessions().remove(&self.id);
        }
    }

    /// Build the Axum router for the MCP SSE transport. No CORS layer: MCP
    /// clients aren't browsers, and cross-origin pages must not reach it.
    pub fn router(state: Arc<SseState>) -> Router {
        Router::new()
            .route("/sse", get(sse_handler))
            .route("/message", post(message_handler))
            .layer(middleware::from_fn_with_state(state.clone(), require_token))
            .with_state(state)
    }

    async fn require_token(
        State(state): State<Arc<SseState>>,
        req: Request,
        next: Next,
    ) -> Response {
        let presented = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());

        match presented {
            Some(token) if state.token_matches(token) => next.run(req).await,
            _ => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                "Unauthorized",
            )
                .into_response(),
        }
    }

    /// `GET /sse` — opens an SSE stream for a new session.
    ///
    /// Sends an initial `endpoint` event with the message URL, then streams
    /// JSON-RPC responses as `message` events.
    async fn sse_handler(State(state): State<Arc<SseState>>) -> Response {
        let session_id = uuid::Uuid::new_v4().to_string();
        let (tx, mut rx) = mpsc::channel::<JsonRpcResponse>(64);

        {
            let mut sessions = state.sessions();
            if sessions.len() >= MAX_SESSIONS {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Too many open MCP sessions",
                )
                    .into_response();
            }
            sessions.insert(session_id.clone(), tx);
        }
        let guard = SessionGuard {
            state: state.clone(),
            id: session_id.clone(),
        };

        let endpoint_url = format!("/message?sessionId={session_id}");

        let stream = async_stream::stream! {
            let _guard = guard;

            // First event: tell the client where to POST messages
            yield Ok::<_, Infallible>(Event::default()
                .event("endpoint")
                .data(endpoint_url));

            // Subsequent events: JSON-RPC responses
            while let Some(response) = rx.recv().await {
                match serde_json::to_string(&response) {
                    Ok(json) => yield Ok(Event::default().event("message").data(json)),
                    Err(e) => tracing::warn!("failed to serialize MCP response: {e}"),
                }
            }
        };

        Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response()
    }

    #[derive(serde::Deserialize)]
    struct MessageQuery {
        #[serde(rename = "sessionId")]
        session_id: String,
    }

    /// `POST /message?sessionId=<id>` — receives a JSON-RPC message and pushes
    /// the response (if any; notifications get none) onto the session's SSE
    /// stream. The response is also returned in the body for convenience.
    async fn message_handler(
        State(state): State<Arc<SseState>>,
        Query(query): Query<MessageQuery>,
        body: Bytes,
    ) -> Response {
        let tx = state.sessions().get(&query.session_id).cloned();
        let Some(tx) = tx else {
            return (
                StatusCode::NOT_FOUND,
                Json(JsonRpcResponse::error(
                    Value::Null,
                    -32000,
                    "Unknown session".to_string(),
                )),
            )
                .into_response();
        };

        let Some(response) = state.server.handle_message(&body).await else {
            return StatusCode::ACCEPTED.into_response();
        };

        if tx.send(response.clone()).await.is_err() {
            tracing::debug!(
                "MCP session {} closed before its response was sent",
                query.session_id
            );
        }

        (StatusCode::ACCEPTED, Json(response)).into_response()
    }

    /// Start the MCP SSE server.
    pub async fn run_sse_server(
        db: aequi_storage::DbPool,
        permissions: Permissions,
        config: SseConfig,
    ) -> anyhow::Result<()> {
        let state = Arc::new(SseState::new(
            McpServer::new(db, permissions),
            &config.token,
        )?);

        let app = router(state);
        let listener = tokio::net::TcpListener::bind(config.addr).await?;
        if !config.addr.ip().is_loopback() {
            tracing::warn!(
                "aequi-mcp SSE server is listening on non-loopback address {}; \
                 the bearer token is sent in clear text unless a TLS proxy is in front",
                config.addr
            );
        }
        tracing::info!("aequi-mcp SSE server listening on {}", config.addr);
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

        const TOKEN: &str = "test-token-123";

        fn req_with_auth(
            method: &str,
            uri: &str,
            auth: Option<&str>,
            body: Body,
        ) -> axum::http::Request<Body> {
            let mut builder = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json");
            if let Some(auth) = auth {
                builder = builder.header("authorization", auth);
            }
            builder.body(body).unwrap()
        }

        fn req(method: &str, uri: &str, body: Body) -> axum::http::Request<Body> {
            req_with_auth(method, uri, Some(&format!("Bearer {TOKEN}")), body)
        }

        async fn test_state() -> Arc<SseState> {
            let db = aequi_storage::create_db(&PathBuf::from(":memory:"))
                .await
                .unwrap();
            aequi_storage::seed_default_accounts(&db).await.unwrap();
            Arc::new(SseState::new(McpServer::new(db, Permissions::default()), TOKEN).unwrap())
        }

        fn setup_session(state: &Arc<SseState>, id: &str) -> mpsc::Receiver<JsonRpcResponse> {
            let (tx, rx) = mpsc::channel(64);
            state.sessions().insert(id.to_string(), tx);
            rx
        }

        fn json_body(value: &serde_json::Value) -> Body {
            Body::from(serde_json::to_string(value).unwrap())
        }

        async fn body_json(response: Response) -> JsonRpcResponse {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
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
            let ct = response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(ct.contains("text/event-stream"));
        }

        #[tokio::test]
        async fn sse_requires_token() {
            let state = test_state().await;
            for auth in [
                None,
                Some("Bearer wrong"),
                Some("Basic dGVzdA=="),
                Some(TOKEN),
            ] {
                let response = router(state.clone())
                    .oneshot(req_with_auth("GET", "/sse", auth, Body::empty()))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{auth:?}");
            }
            assert_eq!(state.session_count(), 0);
        }

        #[tokio::test]
        async fn message_requires_token() {
            let state = test_state().await;
            let mut rx = setup_session(&state, "s0");
            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "tools/call",
                "params": { "name": "aequi_list_accounts", "arguments": {} }
            });
            let response = router(state)
                .oneshot(req_with_auth(
                    "POST",
                    "/message?sessionId=s0",
                    Some("Bearer nope"),
                    json_body(&body),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(rx.try_recv().is_err());
        }

        #[tokio::test]
        async fn bearer_scheme_is_case_insensitive() {
            let state = test_state().await;
            let response = router(state)
                .oneshot(req_with_auth(
                    "GET",
                    "/sse",
                    Some(&format!("bearer {TOKEN}")),
                    Body::empty(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test]
        async fn session_removed_when_stream_dropped() {
            let state = test_state().await;
            let response = router(state.clone())
                .oneshot(req("GET", "/sse", Body::empty()))
                .await
                .unwrap();
            assert_eq!(state.session_count(), 1);
            drop(response);
            assert_eq!(state.session_count(), 0);
        }

        #[tokio::test]
        async fn session_count_is_capped() {
            let state = test_state().await;
            let mut open = Vec::new();
            for _ in 0..MAX_SESSIONS {
                let response = router(state.clone())
                    .oneshot(req("GET", "/sse", Body::empty()))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                open.push(response);
            }
            let response = router(state.clone())
                .oneshot(req("GET", "/sse", Body::empty()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

            open.pop();
            let response = router(state.clone())
                .oneshot(req("GET", "/sse", Body::empty()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
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
                .oneshot(req(
                    "POST",
                    "/message?sessionId=nonexistent",
                    json_body(&body),
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn message_with_valid_session() {
            let state = test_state().await;
            let mut rx = setup_session(&state, "s1");
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
            let pushed = rx.try_recv().unwrap();
            assert_eq!(pushed.id, serde_json::json!(1));
            assert!(pushed.result.is_some());
        }

        #[tokio::test]
        async fn message_tools_list() {
            let state = test_state().await;
            let _rx = setup_session(&state, "s2");
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
            let resp = body_json(response).await;
            let tools = resp.result.unwrap();
            assert_eq!(tools["tools"].as_array().unwrap().len(), 24);
        }

        #[tokio::test]
        async fn message_tool_call() {
            let state = test_state().await;
            let _rx = setup_session(&state, "s3");
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
            let resp = body_json(response).await;
            let result = resp.result.unwrap();
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Checking"));
        }

        #[tokio::test]
        async fn message_unknown_method() {
            let state = test_state().await;
            let _rx = setup_session(&state, "s4");
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
            let resp = body_json(response).await;
            assert!(resp.error.is_some());
            assert_eq!(resp.error.unwrap().code, -32601);
        }

        #[tokio::test]
        async fn notification_gets_no_response() {
            let state = test_state().await;
            let mut rx = setup_session(&state, "s5");
            let app = router(state);

            let body = serde_json::json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            });
            let response = app
                .oneshot(req("POST", "/message?sessionId=s5", json_body(&body)))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(bytes.is_empty());
            assert!(rx.try_recv().is_err());
        }

        #[tokio::test]
        async fn malformed_body_gets_parse_error_with_null_id() {
            let state = test_state().await;
            let mut rx = setup_session(&state, "s6");
            let app = router(state);

            let response = app
                .oneshot(req(
                    "POST",
                    "/message?sessionId=s6",
                    Body::from("{not json"),
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let pushed = rx.try_recv().unwrap();
            assert_eq!(pushed.error.unwrap().code, -32700);
            let raw = serde_json::to_value(body_json(response).await).unwrap();
            assert!(raw.as_object().unwrap().contains_key("id"));
            assert!(raw["id"].is_null());
        }

        #[test]
        fn config_defaults_to_loopback() {
            let c = SseConfig::from_vars(None, None, Some("secret".into())).unwrap();
            assert_eq!(c.addr, "127.0.0.1:8061".parse().unwrap());
        }

        #[test]
        fn config_overrides() {
            let c = SseConfig::from_vars(
                Some("0.0.0.0".into()),
                Some("9000".into()),
                Some("secret".into()),
            )
            .unwrap();
            assert_eq!(c.addr, "0.0.0.0:9000".parse().unwrap());
        }

        #[test]
        fn config_requires_token() {
            assert!(SseConfig::from_vars(None, None, None).is_err());
            assert!(SseConfig::from_vars(None, None, Some("  ".into())).is_err());
        }

        #[test]
        fn config_rejects_bad_values() {
            assert!(
                SseConfig::from_vars(Some("localhost:80".into()), None, Some("s".into())).is_err()
            );
            assert!(SseConfig::from_vars(None, Some("99999".into()), Some("s".into())).is_err());
        }
    }
}
