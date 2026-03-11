use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

mod daimon;
mod error;
mod routes;
mod state;

use state::ServerState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

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

    let state = Arc::new(ServerState { db, api_key });

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
