use std::path::PathBuf;

use anyhow::{Context, Result, bail};

/// The desktop app's bundle identifier; Tauri keeps its data (ledger.db,
/// attachments/) in `<platform data dir>/<identifier>`.
const APP_IDENTIFIER: &str = "com.aequi.app";

enum Transport {
    Stdio,
    #[cfg(feature = "sse")]
    Sse(aequi_mcp::sse::transport::SseConfig),
}

#[tokio::main]
async fn main() -> Result<()> {
    // stdout carries the stdio protocol; all logging goes to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    // Validate the transport configuration before touching the ledger.
    let transport = match std::env::var("AEQUI_MCP_TRANSPORT")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("stdio") => Transport::Stdio,
        #[cfg(feature = "sse")]
        Some("sse") => Transport::Sse(aequi_mcp::sse::transport::SseConfig::from_env()?),
        #[cfg(not(feature = "sse"))]
        Some("sse") => bail!("this aequi-mcp build has no SSE support (build with --features sse)"),
        Some(other) => {
            bail!("unknown AEQUI_MCP_TRANSPORT {other:?}; expected \"stdio\" or \"sse\"")
        }
    };

    let db_path = resolve_db_path()?;
    let db = aequi_storage::create_db(&db_path)
        .await
        .with_context(|| format!("cannot open ledger {}", db_path.display()))?;
    aequi_storage::seed_default_accounts(&db).await?;

    // Static restrictions only; the desktop app's MCP settings are re-read
    // from the ledger on every tool call.
    let permissions = aequi_mcp::permissions::Permissions::default();

    match transport {
        #[cfg(feature = "sse")]
        Transport::Sse(config) => {
            tracing::info!("aequi-mcp server starting on SSE ({})", config.addr);
            aequi_mcp::sse::transport::run_sse_server(db, permissions, config).await?;
        }
        Transport::Stdio => {
            tracing::info!("aequi-mcp server starting on stdio");
            aequi_mcp::server::run_stdio_server(db, permissions).await?;
        }
    }

    Ok(())
}

/// `AEQUI_DB_PATH` if set (created if missing, for development and tests);
/// otherwise the desktop app's ledger, which must already exist so an agent
/// is never silently handed a fresh, empty ledger.
fn resolve_db_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AEQUI_DB_PATH").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let base = directories::BaseDirs::new().context(
        "cannot determine the user data directory; set AEQUI_DB_PATH to the Aequi ledger.db file",
    )?;
    let path = base.data_dir().join(APP_IDENTIFIER).join("ledger.db");
    if !path.is_file() {
        bail!(
            "no Aequi ledger found at {}. Open the Aequi desktop app once to create it, \
             or set AEQUI_DB_PATH to the ledger.db file to use",
            path.display()
        );
    }
    Ok(path)
}
