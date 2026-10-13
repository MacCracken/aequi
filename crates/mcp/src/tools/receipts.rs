use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, ToolOutcome, not_found};
use crate::tools::{Access, ToolRegistry};

/// Largest receipt file accepted for ingestion.
pub const MAX_RECEIPT_BYTES: u64 = 50 * 1024 * 1024;

const ALLOWED_EXTS: [&str; 9] = [
    "jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "pdf",
];

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_ingest_receipt".to_string(),
            description: "Ingest a receipt image or PDF by file path. The file is copied into Aequi's attachments store and queued for review".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute path to the receipt image file (max 50 MB)" },
                    "vendor": { "type": "string", "description": "Vendor name (if known)" },
                    "date": { "type": "string", "description": "Receipt date YYYY-MM-DD (if known)" },
                    "total_cents": { "type": "integer", "minimum": 0, "description": "Total amount in cents (if known)" }
                },
                "required": ["file_path"]
            }),
        },
        Access::Write,
        |db, params| async move { ingest_receipt(&db, &params).await },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_pending_receipts".to_string(),
            description: "List receipts pending review".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let receipts = aequi_storage::get_receipts_pending_review(&db).await?;
            Ok(ToolResult::json(&receipts))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_approve_receipt".to_string(),
            description: "Approve a receipt, optionally linking it to a transaction".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "receipt_id": { "type": "integer" },
                    "transaction_id": { "type": "integer" }
                },
                "required": ["receipt_id"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let args = Args::new(&params);
            let receipt_id = args.id("receipt_id")?;
            let receipt = format!("Receipt {receipt_id}");

            if let Some(tx_id) = args.opt_id("transaction_id")? {
                let exists =
                    sqlx::query_as::<_, (i64,)>("SELECT id FROM transactions WHERE id = ?")
                        .bind(tx_id)
                        .fetch_optional(&db)
                        .await?;
                if exists.is_none() {
                    return Err(ToolError(format!("Transaction {tx_id} not found")));
                }
                aequi_storage::link_receipt_to_transaction(&db, receipt_id, tx_id)
                    .await
                    .map_err(not_found(receipt))?;
            } else {
                aequi_storage::update_receipt_status(&db, receipt_id, "approved")
                    .await
                    .map_err(not_found(receipt))?;
            }
            Ok(ToolResult::text("Receipt approved".to_string()))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_reject_receipt".to_string(),
            description: "Reject a receipt".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "receipt_id": { "type": "integer" } },
                "required": ["receipt_id"]
            }),
        },
        Access::Write,
        |db, params| async move {
            let receipt_id = Args::new(&params).id("receipt_id")?;
            aequi_storage::update_receipt_status(&db, receipt_id, "rejected")
                .await
                .map_err(not_found(format!("Receipt {receipt_id}")))?;
            Ok(ToolResult::text("Receipt rejected".to_string()))
        },
    );
}

async fn ingest_receipt(db: &aequi_storage::DbPool, params: &serde_json::Value) -> ToolOutcome {
    let args = Args::new(params);
    let file_path = args
        .opt_str("file_path")?
        .ok_or_else(|| ToolError::new("file_path is required"))?;

    let path = Path::new(file_path);
    if !path.is_absolute() {
        return Err(ToolError::new("file_path must be an absolute path"));
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(ToolError::new("Path traversal not allowed"));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !ALLOWED_EXTS.contains(&ext.as_str()) {
        return Err(ToolError(format!("Unsupported file type: .{ext}")));
    }

    let vendor = args.opt_str("vendor")?;
    let date = args.opt_date("date")?;
    let total_cents = args.opt_cents("total_cents")?;
    if total_cents.is_some_and(|c| c < 0) {
        return Err(ToolError::new("total_cents must not be negative"));
    }

    // Check the type before opening: opening a FIFO would block.
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| ToolError(format!("Cannot read file: {e}")))?;
    check_receipt_metadata(&meta)?;
    let mut src = tokio::fs::File::open(path)
        .await
        .map_err(|e| ToolError(format!("Cannot read file: {e}")))?;
    // Re-check on the opened handle in case the path was swapped meanwhile.
    let meta = src
        .metadata()
        .await
        .map_err(|e| ToolError(format!("Cannot read file: {e}")))?;
    check_receipt_metadata(&meta)?;

    let attachments_dir = attachments_dir(db).await?;
    tokio::fs::create_dir_all(&attachments_dir)
        .await
        .map_err(|e| ToolError(format!("Cannot create attachments directory: {e}")))?;

    let tmp_path = attachments_dir.join(temp_name());
    let hash = match copy_and_hash(&mut src, &tmp_path).await {
        Ok(hash) => hash,
        Err(e) => {
            remove_quietly(&tmp_path).await;
            return Err(e);
        }
    };

    if let Some(existing) = aequi_storage::check_receipt_duplicate(db, &hash).await? {
        remove_quietly(&tmp_path).await;
        return Err(ToolError(format!(
            "Duplicate receipt (existing id: {existing})"
        )));
    }

    // Content-addressed layout shared with the desktop app's OCR pipeline:
    // <attachments>/<first 2 hex chars>/<hash>.<ext>
    let dest = attachments_dir
        .join(&hash[..2])
        .join(format!("{hash}.{ext}"));
    let created = match place_attachment(&tmp_path, &dest).await {
        Ok(created) => created,
        Err(e) => {
            remove_quietly(&tmp_path).await;
            return Err(e);
        }
    };
    let dest_str = dest
        .to_str()
        .ok_or_else(|| ToolError::new("Attachment path is not valid UTF-8"))?;

    let inserted = aequi_storage::insert_receipt(
        db,
        &hash,
        &ext,
        dest_str,
        None,
        vendor,
        date,
        total_cents,
        None,
        None,
        None,
        0.0,
    )
    .await;
    let id = match inserted {
        Ok(id) => id,
        Err(e) => {
            if created {
                remove_quietly(&dest).await;
            }
            return Err(e.into());
        }
    };

    Ok(ToolResult::text(
        json!({
            "receipt_id": id,
            "file_hash": hash,
            "attachment_path": dest_str,
            "status": "pending_review"
        })
        .to_string(),
    ))
}

fn check_receipt_metadata(meta: &std::fs::Metadata) -> Result<(), ToolError> {
    if !meta.is_file() {
        return Err(ToolError::new("file_path must point to a regular file"));
    }
    if meta.len() > MAX_RECEIPT_BYTES {
        return Err(ToolError(format!(
            "File is too large ({} bytes); the limit is {} MB",
            meta.len(),
            MAX_RECEIPT_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// The desktop app keeps attachments next to its ledger:
/// `<directory of ledger.db>/attachments`.
async fn attachments_dir(db: &aequi_storage::DbPool) -> Result<PathBuf, ToolError> {
    let (file,) =
        sqlx::query_as::<_, (String,)>("SELECT file FROM pragma_database_list WHERE name = 'main'")
            .fetch_one(db)
            .await?;
    let parent = Path::new(&file)
        .parent()
        .filter(|_| !file.is_empty())
        .ok_or_else(|| {
            ToolError::new("Receipts can't be stored: the ledger is not a file-backed database")
        })?;
    Ok(parent.join("attachments"))
}

fn temp_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        ".mcp-ingest-{}-{nanos}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Stream `src` into a new file at `tmp_path`, returning the SHA-256 hex
/// digest. Enforces the size cap while reading, in case the file grows.
async fn copy_and_hash(src: &mut tokio::fs::File, tmp_path: &Path) -> Result<String, ToolError> {
    let io_err = |e: std::io::Error| ToolError(format!("Cannot store receipt: {e}"));
    let mut out = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp_path)
        .await
        .map_err(io_err)?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = src
            .read(&mut buf)
            .await
            .map_err(|e| ToolError(format!("Cannot read file: {e}")))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_RECEIPT_BYTES {
            return Err(ToolError(format!(
                "File is too large; the limit is {} MB",
                MAX_RECEIPT_BYTES / (1024 * 1024)
            )));
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).await.map_err(io_err)?;
    }
    out.flush().await.map_err(io_err)?;
    out.sync_all().await.map_err(io_err)?;
    Ok(hex::encode(hasher.finalize()))
}

/// Move the temp file to its content-addressed name. Returns whether a new
/// file was created (false if identical content was already stored).
async fn place_attachment(tmp_path: &Path, dest: &Path) -> Result<bool, ToolError> {
    let io_err = |e: std::io::Error| ToolError(format!("Cannot store receipt: {e}"));
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(io_err)?;
    }
    if tokio::fs::try_exists(dest).await.map_err(io_err)? {
        remove_quietly(tmp_path).await;
        return Ok(false);
    }
    tokio::fs::rename(tmp_path, dest).await.map_err(io_err)?;
    Ok(true)
}

async fn remove_quietly(path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("failed to remove {}: {e}", path.display());
    }
}
