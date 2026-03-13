use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::state::ServerState;

const DEFAULT_DAIMON_URL: &str = "http://127.0.0.1:8090";
const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const REGISTER_RETRY_DELAY: Duration = Duration::from_secs(10);
const MAX_REGISTER_RETRIES: u32 = 5;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RegisterRequest {
    name: String,
    capabilities: Vec<String>,
    resource_needs: ResourceNeeds,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ResourceNeeds {
    memory_mb: u32,
    cpu_cores: f32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RegisterResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub registered_at: String,
}

#[derive(Debug, Serialize)]
struct DashboardSyncRequest {
    source: String,
    agents: Vec<AgentStatus>,
    session: SessionInfo,
    metrics: serde_json::Value,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct AgentStatus {
    name: String,
    status: String,
    current_task: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    started_at: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DashboardSyncResponse {
    pub status: String,
    pub snapshot_id: Option<String>,
    pub agents_synced: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DiscoverResponse {
    pub capabilities: Option<Vec<String>>,
    pub endpoints: Option<serde_json::Value>,
    pub companion_services: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct DaimonClient {
    http: reqwest::Client,
    base_url: String,
}

impl DaimonClient {
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Discover daimon capabilities and companion service URLs.
    pub async fn discover(&self) -> Result<DiscoverResponse, reqwest::Error> {
        self.http
            .get(format!("{}/v1/discover", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// Register aequi as an MCP agent with the daimon orchestrator.
    pub async fn register_agent(&self) -> Result<RegisterResponse, reqwest::Error> {
        let body = RegisterRequest {
            name: "aequi".to_string(),
            capabilities: vec![
                "bookkeeping".to_string(),
                "tax-estimation".to_string(),
                "invoicing".to_string(),
                "receipt-ocr".to_string(),
                "bank-import".to_string(),
                "mcp-server".to_string(),
            ],
            resource_needs: ResourceNeeds {
                memory_mb: 512,
                cpu_cores: 1.0,
            },
            metadata: serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "port": 8060,
                "transport": "stdio",
                "tools_count": 24,
            }),
        };

        self.http
            .post(format!("{}/v1/agents/register", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// Send a dashboard sync heartbeat with current status.
    pub async fn dashboard_sync(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<DashboardSyncResponse, reqwest::Error> {
        let body = DashboardSyncRequest {
            source: "aequi".to_string(),
            agents: vec![AgentStatus {
                name: "aequi".to_string(),
                status: "running".to_string(),
                current_task: None,
            }],
            session: SessionInfo {
                id: session_id.to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
            },
            metrics: serde_json::json!({
                "uptime_seconds": 0,
                "tools_available": 24,
            }),
            metadata: serde_json::json!({
                "agent_id": agent_id,
            }),
        };

        self.http
            .post(format!("{}/v1/dashboard/sync", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}

// ---------------------------------------------------------------------------
// Background task
// ---------------------------------------------------------------------------

/// Spawn the daimon integration background task.
///
/// This task:
/// 1. Calls `GET /v1/discover` to verify daimon is reachable
/// 2. Registers aequi as an agent via `POST /v1/agents/register`
/// 3. Sends periodic `POST /v1/dashboard/sync` heartbeats
///
/// If daimon is unreachable, the task retries registration with backoff.
/// The task shuts down when the `shutdown` receiver fires.
pub fn spawn_daimon_task(
    _state: Arc<ServerState>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let daimon_url =
        std::env::var("AEQUI_DAIMON_URL").unwrap_or_else(|_| DEFAULT_DAIMON_URL.to_string());

    tokio::spawn(async move {
        let client = DaimonClient::new(&daimon_url);
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut shutdown = shutdown;

        // Step 1: Discover
        match client.discover().await {
            Ok(resp) => {
                tracing::info!(
                    "daimon discover OK: {:?} capabilities",
                    resp.capabilities.as_ref().map(|c| c.len())
                );
            }
            Err(e) => {
                tracing::warn!("daimon discover failed (will still attempt registration): {e}");
            }
        }

        // Step 2: Register with retries
        let mut agent_id = None;
        for attempt in 1..=MAX_REGISTER_RETRIES {
            match client.register_agent().await {
                Ok(resp) => {
                    tracing::info!(
                        "registered with daimon as agent '{}' (id={})",
                        resp.name,
                        resp.id
                    );
                    agent_id = Some(resp.id);
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        "daimon registration attempt {attempt}/{MAX_REGISTER_RETRIES} failed: {e}"
                    );
                    if attempt < MAX_REGISTER_RETRIES {
                        tokio::select! {
                            _ = tokio::time::sleep(REGISTER_RETRY_DELAY) => {}
                            _ = shutdown.changed() => return,
                        }
                    }
                }
            }
        }

        let agent_id = match agent_id {
            Some(id) => id,
            None => {
                tracing::warn!("could not register with daimon after {MAX_REGISTER_RETRIES} attempts; daimon sync disabled");
                return;
            }
        };

        // Step 3: Periodic dashboard sync
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match client.dashboard_sync(&agent_id, &session_id).await {
                        Ok(resp) => {
                            tracing::debug!("daimon sync OK: status={}", resp.status);
                        }
                        Err(e) => {
                            tracing::warn!("daimon sync failed: {e}");
                        }
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("daimon sync task shutting down");
                    return;
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation() {
        let client = DaimonClient::new("http://localhost:8090");
        assert_eq!(client.base_url, "http://localhost:8090");
    }

    #[test]
    fn client_strips_trailing_slash() {
        let client = DaimonClient::new("http://localhost:8090/");
        assert_eq!(client.base_url, "http://localhost:8090");
    }

    #[test]
    fn register_request_serializes() {
        let req = RegisterRequest {
            name: "aequi".to_string(),
            capabilities: vec!["bookkeeping".to_string()],
            resource_needs: ResourceNeeds {
                memory_mb: 512,
                cpu_cores: 1.0,
            },
            metadata: serde_json::json!({"version": "1.0"}),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"name\":\"aequi\""));
        assert!(json.contains("\"memory_mb\":512"));
    }

    #[test]
    fn dashboard_sync_request_serializes() {
        let req = DashboardSyncRequest {
            source: "aequi".to_string(),
            agents: vec![AgentStatus {
                name: "aequi".to_string(),
                status: "running".to_string(),
                current_task: None,
            }],
            session: SessionInfo {
                id: "test-session".to_string(),
                started_at: "2026-03-10T00:00:00Z".to_string(),
            },
            metrics: serde_json::json!({}),
            metadata: serde_json::json!({}),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"source\":\"aequi\""));
        assert!(json.contains("\"status\":\"running\""));
    }

    #[test]
    fn discover_response_deserializes() {
        let json = r#"{"capabilities":["agents","rag"],"endpoints":{},"companion_services":{}}"#;
        let resp: DiscoverResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.capabilities.unwrap().len(), 2);
    }

    #[test]
    fn register_response_deserializes() {
        let json = r#"{"id":"abc-123","name":"aequi","status":"registered","registered_at":"2026-03-10T00:00:00Z"}"#;
        let resp: RegisterResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "abc-123");
        assert_eq!(resp.name, "aequi");
    }

    #[test]
    fn dashboard_sync_response_deserializes() {
        let json = r#"{"status":"ok","snapshot_id":"snap-1","agents_synced":1}"#;
        let resp: DashboardSyncResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.agents_synced, Some(1));
    }

    #[tokio::test]
    async fn spawn_task_shuts_down_on_signal() {
        let state = Arc::new(ServerState {
            db: aequi_storage::create_db(&std::path::PathBuf::from(":memory:"))
                .await
                .unwrap(),
            api_key: None,
            email_config: None,
            oidc: None,
        });

        let (tx, rx) = watch::channel(false);

        // Set a bogus URL so registration fails fast
        std::env::set_var("AEQUI_DAIMON_URL", "http://127.0.0.1:1");
        let handle = spawn_daimon_task(state, rx);

        // Signal shutdown after a brief delay
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tx.send(true);

        // Task should complete within a reasonable time
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("task did not shut down in time")
            .expect("task panicked");
    }
}
