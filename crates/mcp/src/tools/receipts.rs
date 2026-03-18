use serde_json::json;

use crate::protocol::{ToolDefinition, ToolResult};
use crate::tools::ToolRegistry;

use sha2::{Digest, Sha256};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolDefinition {
            name: "aequi_ingest_receipt".to_string(),
            description: "Ingest a receipt image by file path for OCR processing".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute path to the receipt image file" },
                    "vendor": { "type": "string", "description": "Vendor name (if known)" },
                    "date": { "type": "string", "description": "Receipt date YYYY-MM-DD (if known)" },
                    "total_cents": { "type": "integer", "description": "Total amount in cents (if known)" }
                },
                "required": ["file_path"]
            }),
        },
        true,
        |db, params| async move {
            let file_path = params.get("file_path").and_then(|v| v.as_str()).unwrap_or("");

            // Security: validate file path
            if file_path.is_empty() {
                return ToolResult::error("file_path is required".to_string());
            }
            let path = std::path::Path::new(file_path);
            if !path.is_absolute() {
                return ToolResult::error("file_path must be an absolute path".to_string());
            }
            // Reject path traversal
            if file_path.contains("..") {
                return ToolResult::error("Path traversal not allowed".to_string());
            }
            // Validate file extension is an image type
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let allowed_exts = ["jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "pdf"];
            if !allowed_exts.contains(&ext.to_lowercase().as_str()) {
                return ToolResult::error(format!("Unsupported file type: .{ext}"));
            }

            let vendor = params.get("vendor").and_then(|v| v.as_str());
            let date = params.get("date").and_then(|v| v.as_str());
            let total_cents = params.get("total_cents").and_then(|v| v.as_i64());

            let data = match tokio::fs::read(file_path).await {
                Ok(d) => d,
                Err(e) => return ToolResult::error(format!("Cannot read file: {e}")),
            };

            let mut hasher = Sha256::new();
            hasher.update(&data);
            let hash = format!("{:x}", hasher.finalize());

            if let Ok(Some(existing)) = aequi_storage::check_receipt_duplicate(&db, &hash).await {
                return ToolResult::error(format!("Duplicate receipt (existing id: {existing})"));
            }

            let ext = std::path::Path::new(file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg");

            match aequi_storage::insert_receipt(
                &db, &hash, ext, file_path, None, vendor, date,
                total_cents, None, None, None, 0.0,
            ).await {
                Ok(id) => ToolResult::text(json!({
                    "receipt_id": id,
                    "file_hash": hash,
                    "status": "pending_review"
                }).to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_get_pending_receipts".to_string(),
            description: "List receipts pending review".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        false,
        |db, _params| async move {
            match aequi_storage::get_receipts_pending_review(&db).await {
                Ok(receipts) => ToolResult::text(serde_json::to_string_pretty(&receipts).unwrap()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );

    registry.register(
        ToolDefinition {
            name: "aequi_approve_receipt".to_string(),
            description: "Approve a receipt".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "receipt_id": { "type": "integer" },
                    "transaction_id": { "type": "integer" }
                },
                "required": ["receipt_id"]
            }),
        },
        true,
        |db, params| async move {
            let receipt_id = params
                .get("receipt_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let transaction_id = params.get("transaction_id").and_then(|v| v.as_i64());

            let result = if let Some(tx_id) = transaction_id {
                aequi_storage::link_receipt_to_transaction(&db, receipt_id, tx_id).await
            } else {
                aequi_storage::update_receipt_status(&db, receipt_id, "approved").await
            };

            match result {
                Ok(()) => ToolResult::text("Receipt approved".to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
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
        true,
        |db, params| async move {
            let receipt_id = params
                .get("receipt_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            match aequi_storage::update_receipt_status(&db, receipt_id, "rejected").await {
                Ok(()) => ToolResult::text("Receipt rejected".to_string()),
                Err(e) => ToolResult::error(e.to_string()),
            }
        },
    );
}
