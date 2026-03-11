use std::path::PathBuf;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let db_path = std::env::var("AEQUI_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("aequi.db"));

    let db = aequi_storage::create_db(&db_path).await?;
    aequi_storage::seed_default_accounts(&db).await?;

    let permissions = aequi_mcp::permissions::Permissions::default();

    tracing::info!("aequi-mcp server starting on stdio");
    aequi_mcp::server::run_stdio_server(db, permissions).await?;

    Ok(())
}
