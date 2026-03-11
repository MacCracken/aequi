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

    let transport = std::env::var("AEQUI_MCP_TRANSPORT").unwrap_or_else(|_| "stdio".to_string());

    match transport.as_str() {
        #[cfg(feature = "sse")]
        "sse" => {
            let port: u16 = std::env::var("AEQUI_MCP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8061);
            tracing::info!("aequi-mcp server starting on SSE (port {port})");
            aequi_mcp::sse::transport::run_sse_server(db, permissions, port).await?;
        }
        "stdio" | _ => {
            tracing::info!("aequi-mcp server starting on stdio");
            aequi_mcp::server::run_stdio_server(db, permissions).await?;
        }
    }

    Ok(())
}
