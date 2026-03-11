use aequi_storage::DbPool;
use sha2::{Digest, Sha256};

pub async fn log_tool_call(db: &DbPool, tool: &str, input: &str, outcome: &str) {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    let _ = aequi_storage::insert_audit_log(db, tool, Some(&hash), outcome, None).await;
}
