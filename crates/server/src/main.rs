use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

mod daimon;
mod error;
pub(crate) mod oidc;
mod routes;
mod state;

use state::ServerState;

#[tokio::main]
async fn main() -> Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    // AEQUI_LOG_FORMAT=json enables structured Bunyan-compatible JSON logging
    if std::env::var("AEQUI_LOG_FORMAT").as_deref() == Ok("json") {
        let formatting_layer =
            tracing_bunyan_formatter::BunyanFormattingLayer::new("aequi-server".into(), std::io::stdout);
        let json_fields_layer = tracing_bunyan_formatter::JsonStorageLayer;

        tracing_subscriber::registry()
            .with(filter)
            .with(json_fields_layer)
            .with(formatting_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .init();
    }

    let db_path = std::env::var("AEQUI_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("aequi.db"));
    let port: u16 = std::env::var("AEQUI_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8060);
    let api_key = std::env::var("AEQUI_API_KEY").ok();

    let db = aequi_storage::create_db(&db_path).await?;
    aequi_storage::seed_default_accounts(&db).await?;

    // Load email config from AEQUI_EMAIL_CONFIG env var (JSON string)
    let email_config: Option<aequi_email::EmailConfig> =
        std::env::var("AEQUI_EMAIL_CONFIG")
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());

    if email_config.is_some() {
        tracing::info!("Email delivery configured");
    }

    // Load OIDC config from AEQUI_OIDC_CONFIG env var (JSON string)
    let oidc: Option<oidc::JwksCache> = std::env::var("AEQUI_OIDC_CONFIG")
        .ok()
        .and_then(|json| serde_json::from_str::<oidc::OidcConfig>(&json).ok())
        .map(oidc::JwksCache::new);

    if oidc.is_some() {
        tracing::info!("OIDC authentication configured");
    }

    let stripe_webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").ok();
    if stripe_webhook_secret.is_some() {
        tracing::info!("Stripe webhook listener configured");
    }

    // Load Plaid config from AEQUI_PLAID_CONFIG env var (JSON string)
    let plaid_config: Option<aequi_import::plaid::PlaidConfig> =
        std::env::var("AEQUI_PLAID_CONFIG")
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());

    if plaid_config.is_some() {
        tracing::info!("Plaid bank sync configured");
    }

    let state = Arc::new(ServerState {
        db,
        api_key,
        email_config,
        oidc,
        stripe_webhook_secret,
        plaid_config,
    });

    // Shutdown signal for background tasks
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn daimon integration (non-blocking — server starts regardless)
    let daimon_handle = daimon::spawn_daimon_task(Arc::clone(&state), shutdown_rx);

    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("aequi-server listening on port {port}");
    axum::serve(listener, app).await?;

    // Server stopped — shut down background tasks
    let _ = shutdown_tx.send(true);
    let _ = daimon_handle.await;

    Ok(())
}
