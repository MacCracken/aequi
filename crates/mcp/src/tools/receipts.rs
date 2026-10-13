use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::protocol::{EmbeddedResource, ToolContent, ToolDefinition, ToolResult};
use crate::tools::args::{Args, ToolError, ToolOutcome, not_found};
use crate::tools::{Access, ToolRegistry};

/// Largest receipt file accepted for ingestion.
pub const MAX_RECEIPT_BYTES: u64 = 50 * 1024 * 1024;

/// Largest receipt file returned by `aequi_get_receipt_image`; base64 makes
/// the response about a third larger.
pub const MAX_RECEIPT_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

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
            description: "List receipts pending review, newest first, with their current fields (vendor, receipt_date, total/subtotal/tax cents, payment_method), confidence (0-1), status and any OCR text. To fill in a receipt's fields, view it with aequi_get_receipt_image and save what you read with aequi_update_receipt_fields. Approving a receipt is done by a person in the Aequi app; there is no tool for it".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Access::Read,
        |db, _params| async move {
            let receipts = aequi_storage::get_receipts_pending_review(&db).await?;
            let rows: Vec<_> = receipts
                .iter()
                .map(|r| {
                    let mut row = receipt_fields_json(r);
                    row["file_ext"] = json!(r.file_ext);
                    row["ocr_text"] = json!(r.ocr_text);
                    row["created_at"] = json!(r.created_at);
                    row
                })
                .collect();
            Ok(ToolResult::json(&rows))
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_receipt_image".to_string(),
            description: "Get a receipt's scanned image (or PDF) so you can read it, plus the receipt's current fields. Use this before aequi_update_receipt_fields".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "receipt_id": { "type": "integer" } },
                "required": ["receipt_id"]
            }),
        },
        Access::Read,
        |db, params| async move { get_receipt_image(&db, &params).await },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_update_receipt_fields".to_string(),
            description: "Fill in the fields of a receipt that is pending review. First look at the receipt with aequi_get_receipt_image and enter only values you can read on it; don't guess. Omitted fields keep their current values. Amounts are whole cents. The receipt stays pending review: a person checks the values and approves it in the Aequi app; agents can't approve receipts".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "receipt_id": { "type": "integer" },
                    "vendor": { "type": "string", "description": "Merchant name as printed on the receipt" },
                    "receipt_date": { "type": "string", "description": "Purchase date YYYY-MM-DD" },
                    "total_cents": { "type": "integer", "minimum": 0, "description": "Amount paid, including tax and tip" },
                    "subtotal_cents": { "type": "integer", "minimum": 0, "description": "Amount before tax" },
                    "tax_cents": { "type": "integer", "minimum": 0, "description": "Sales tax" },
                    "payment_method": { "type": "string", "description": "e.g. \"Visa ending 1234\", \"cash\"" },
                    "confidence": { "type": "number", "minimum": 0, "maximum": 1, "description": "How sure you are of these values, from 0 to 1" }
                },
                "required": ["receipt_id", "confidence"]
            }),
        },
        Access::Write,
        |db, params| async move { update_receipt_fields(&db, &params).await },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_reject_receipt".to_string(),
            description: "Reject a receipt, e.g. an obvious duplicate or a file that isn't a receipt. Approval is not available here: a person approves receipts in the Aequi app".to_string(),
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

/// The fields an agent fills in, plus what it needs to decide what to do.
fn receipt_fields_json(r: &aequi_storage::ReceiptRecord) -> Value {
    json!({
        "receipt_id": r.id,
        "status": r.status,
        "vendor": r.vendor,
        "receipt_date": r.receipt_date,
        "total_cents": r.total_cents,
        "subtotal_cents": r.subtotal_cents,
        "tax_cents": r.tax_cents,
        "payment_method": r.payment_method,
        "confidence": r.confidence,
        "transaction_id": r.transaction_id,
    })
}

async fn get_receipt_image(db: &aequi_storage::DbPool, params: &Value) -> ToolOutcome {
    let receipt_id = Args::new(params).id("receipt_id")?;
    let receipt = aequi_storage::get_receipt_by_id(db, receipt_id)
        .await?
        .ok_or_else(|| ToolError(format!("Receipt {receipt_id} not found")))?;

    let bytes = read_attachment(db, receipt_id, &receipt.attachment_path).await?;
    let kind = sniff_file_type(&bytes).ok_or_else(|| {
        ToolError(format!(
            "Receipt {receipt_id}'s file is not a supported image (JPEG, PNG, GIF, WebP, TIFF, BMP) or PDF"
        ))
    })?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let mut info = receipt_fields_json(&receipt);
    info["mime_type"] = json!(kind.mime_type);
    info["size_bytes"] = json!(bytes.len());
    let file_block = if kind.is_image {
        ToolContent::Image {
            data,
            mime_type: kind.mime_type.to_string(),
        }
    } else {
        ToolContent::Resource {
            resource: EmbeddedResource {
                uri: format!("aequi://receipts/{receipt_id}"),
                mime_type: kind.mime_type.to_string(),
                blob: data,
            },
        }
    };
    Ok(ToolResult::new(vec![
        ToolContent::text(info.to_string()),
        file_block,
    ]))
}

/// Read a stored receipt file, refusing anything outside the ledger's
/// attachments directory (after resolving symlinks), anything that isn't a
/// regular file, and files over [`MAX_RECEIPT_IMAGE_BYTES`].
async fn read_attachment(
    db: &aequi_storage::DbPool,
    receipt_id: i64,
    attachment_path: &str,
) -> Result<Vec<u8>, ToolError> {
    let missing =
        |e: std::io::Error| ToolError(format!("Receipt {receipt_id}'s file can't be read: {e}"));
    let root = attachments_dir(db).await?;
    let root = tokio::fs::canonicalize(&root).await.map_err(missing)?;
    let path = tokio::fs::canonicalize(attachment_path)
        .await
        .map_err(missing)?;
    if !path.starts_with(&root) {
        return Err(ToolError(format!(
            "Receipt {receipt_id}'s file is outside Aequi's attachments directory; refusing to read it"
        )));
    }

    // Check the type before opening: opening a FIFO would block.
    let meta = tokio::fs::metadata(&path).await.map_err(missing)?;
    check_image_metadata(receipt_id, &meta)?;
    let file = tokio::fs::File::open(&path).await.map_err(missing)?;
    let meta = file.metadata().await.map_err(missing)?;
    check_image_metadata(receipt_id, &meta)?;

    // Cap the read too, in case the file grows after the check.
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(missing)?;
    if bytes.len() as u64 > MAX_RECEIPT_IMAGE_BYTES {
        return Err(image_too_large(receipt_id));
    }
    Ok(bytes)
}

fn check_image_metadata(receipt_id: i64, meta: &std::fs::Metadata) -> Result<(), ToolError> {
    if !meta.is_file() {
        return Err(ToolError(format!(
            "Receipt {receipt_id}'s attachment is not a regular file"
        )));
    }
    if meta.len() > MAX_RECEIPT_IMAGE_BYTES {
        return Err(image_too_large(receipt_id));
    }
    Ok(())
}

fn image_too_large(receipt_id: i64) -> ToolError {
    ToolError(format!(
        "Receipt {receipt_id}'s file is larger than {} MB and can't be returned",
        MAX_RECEIPT_IMAGE_BYTES / (1024 * 1024)
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct FileKind {
    mime_type: &'static str,
    is_image: bool,
}

/// Identify a receipt file from its leading bytes; the stored extension came
/// from the original file name and isn't trusted.
fn sniff_file_type(bytes: &[u8]) -> Option<FileKind> {
    let image = |mime_type| {
        Some(FileKind {
            mime_type,
            is_image: true,
        })
    };
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\xFF\xD8\xFF") {
        image("image/jpeg")
    } else if starts(b"\x89PNG\r\n\x1a\n") {
        image("image/png")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        image("image/gif")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        image("image/webp")
    } else if starts(b"II*\0") || starts(b"MM\0*") {
        image("image/tiff")
    } else if starts(b"BM") && bytes.len() >= 14 {
        // 14 bytes: the BMP file header.
        image("image/bmp")
    } else if starts(b"%PDF-") {
        Some(FileKind {
            mime_type: "application/pdf",
            is_image: false,
        })
    } else {
        None
    }
}

async fn update_receipt_fields(db: &aequi_storage::DbPool, params: &Value) -> ToolOutcome {
    let args = Args::new(params);
    let receipt_id = args.id("receipt_id")?;
    let confidence = args.f64("confidence")?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(ToolError(format!(
            "Argument confidence must be between 0 and 1, got {confidence}"
        )));
    }
    let amount = |name: &str| -> Result<Option<i64>, ToolError> {
        match args.opt_cents(name)? {
            Some(c) if c < 0 => Err(ToolError(format!("{name} must not be negative"))),
            other => Ok(other),
        }
    };
    let vendor = args.opt_str("vendor")?;
    let receipt_date = args.opt_date("receipt_date")?;
    let total_cents = amount("total_cents")?;
    let subtotal_cents = amount("subtotal_cents")?;
    let tax_cents = amount("tax_cents")?;
    let payment_method = args.opt_str("payment_method")?;

    let current = aequi_storage::get_receipt_by_id(db, receipt_id)
        .await?
        .ok_or_else(|| ToolError(format!("Receipt {receipt_id} not found")))?;
    let fields = aequi_storage::ReceiptFields {
        vendor: vendor.map(str::to_string).or(current.vendor),
        receipt_date: receipt_date.map(str::to_string).or(current.receipt_date),
        total_cents: total_cents.or(current.total_cents),
        subtotal_cents: subtotal_cents.or(current.subtotal_cents),
        tax_cents: tax_cents.or(current.tax_cents),
        payment_method: payment_method
            .map(str::to_string)
            .or(current.payment_method),
    };
    aequi_storage::update_receipt_fields(db, receipt_id, &fields, confidence)
        .await
        .map_err(not_found(format!("Receipt {receipt_id}")))?;

    let updated = aequi_storage::get_receipt_by_id(db, receipt_id)
        .await?
        .ok_or_else(|| ToolError(format!("Receipt {receipt_id} not found")))?;
    let mut out = receipt_fields_json(&updated);
    out["note"] = json!(
        "Saved. The receipt stays pending review until a person approves it in the Aequi app."
    );
    Ok(ToolResult::json(&out))
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
            ToolError::new(
                "Receipt files are unavailable: the ledger is not a file-backed database",
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_receipt_formats_from_bytes() {
        let mime = |bytes: &[u8]| sniff_file_type(bytes).map(|k| (k.mime_type, k.is_image));
        assert_eq!(
            mime(b"\xFF\xD8\xFF\xE0\0\x10JFIF"),
            Some(("image/jpeg", true))
        );
        assert_eq!(mime(b"\x89PNG\r\n\x1a\n\0\0"), Some(("image/png", true)));
        assert_eq!(mime(b"GIF89a\x01\0"), Some(("image/gif", true)));
        assert_eq!(mime(b"GIF87a\x01\0"), Some(("image/gif", true)));
        assert_eq!(mime(b"RIFF\x24\0\0\0WEBPVP8 "), Some(("image/webp", true)));
        assert_eq!(mime(b"II*\0\x08\0\0\0"), Some(("image/tiff", true)));
        assert_eq!(mime(b"MM\0*\0\0\0\x08"), Some(("image/tiff", true)));
        assert_eq!(
            mime(b"BM\x36\0\0\0\0\0\0\0\x36\0\0\0"),
            Some(("image/bmp", true))
        );
        assert_eq!(mime(b"%PDF-1.7\n"), Some(("application/pdf", false)));

        for bytes in [
            &b""[..],
            b"\xFF\xD8",
            b"\x89PNG\r\n",
            b"GIF88a",
            b"RIFF\0\0\0\0WAVEfmt ",
            b"BM",
            b"plain text",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        ] {
            assert_eq!(mime(bytes), None, "{bytes:?}");
        }
    }
}
