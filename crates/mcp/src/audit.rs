use aequi_storage::DbPool;
use sha2::{Digest, Sha256};

pub async fn log_tool_call(db: &DbPool, tool: &str, input: &str, outcome: &str) {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let hash = hex::encode(hasher.finalize());

    // The tool call has already run, so a failed audit write can't undo it;
    // report it on stderr rather than to the client.
    if let Err(e) = aequi_storage::insert_audit_log(db, tool, Some(&hash), outcome, None).await {
        tracing::warn!("failed to write MCP audit log entry for {tool}: {e}");
    }
}
