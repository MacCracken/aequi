use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

mod commands;

pub struct AppState {
    pub db: aequi_storage::DbPool,
    /// Root of the content-addressed attachment store (~/.aequi/attachments/).
    pub attachments_dir: PathBuf,
    /// Sender for the receipt intake pipeline — drop a file path to enqueue.
    pub receipt_tx: mpsc::Sender<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let project_dirs = directories::ProjectDirs::from("com", "aequi", "Aequi")
        .expect("Failed to get app directory");
    let data_dir = project_dirs.data_dir().to_path_buf();

    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("ledger.db");
    let db = aequi_storage::create_db(&db_path)
        .await
        .expect("Failed to create database");

    aequi_storage::seed_default_accounts(&db)
        .await
        .expect("Failed to seed default accounts");

    let attachments_dir = data_dir.join("attachments");
    let intake_dir = data_dir.join("intake");
    std::fs::create_dir_all(&attachments_dir).expect("Failed to create attachments directory");
    std::fs::create_dir_all(&intake_dir).expect("Failed to create intake directory");

    // ── Receipt intake pipeline ───────────────────────────────────────────────
    // The channel bridges the notify watcher thread and the async processor.
    let (receipt_tx, mut receipt_rx) = mpsc::channel::<PathBuf>(64);

    let db_for_pipeline = db.clone();
    let attachments_for_pipeline = attachments_dir.clone();

    tokio::spawn(async move {
        use aequi_ocr::{MockRecognizer, ReceiptPipeline};

        // TODO(phase-3): Replace MockRecognizer with TesseractRecognizer when the
        // `tesseract` feature is enabled and data path is configured.
        let pipeline = ReceiptPipeline::new(
            MockRecognizer::new(""),
            attachments_for_pipeline,
        );

        while let Some(path) = receipt_rx.recv().await {
            tracing::info!("Processing receipt: {}", path.display());
            match pipeline.process_file(&path).await {
                Ok(result) => {
                    let e = &result.extracted;
                    let ext = path
                        .extension()
                        .and_then(|x| x.to_str())
                        .unwrap_or("bin");
                    let _ = aequi_storage::insert_receipt(
                        &db_for_pipeline,
                        &result.hash_hex,
                        ext,
                        result.attachment_path.to_str().unwrap_or(""),
                        Some(&result.ocr_text),
                        e.vendor.as_ref().map(|f| f.value.as_str()),
                        e.date.as_ref().map(|f| f.value.to_string()).as_deref(),
                        e.total_cents.as_ref().map(|f| f.value),
                        e.subtotal_cents.as_ref().map(|f| f.value),
                        e.tax_cents.as_ref().map(|f| f.value),
                        e.payment_method.as_ref().map(|f| f.value.to_string()).as_deref(),
                        e.confidence as f64,
                    )
                    .await;
                    tracing::info!("Receipt stored: {}", result.hash_hex);
                }
                Err(e) => {
                    tracing::warn!("Receipt pipeline error: {e}");
                }
            }
        }
    });

    // ── Watch folder (desktop) ────────────────────────────────────────────────
    // The watcher must be kept alive for the duration of the app.
    let receipt_tx_for_watcher = receipt_tx.clone();
    let _watcher = aequi_ocr::pipeline::spawn_intake_watcher(&intake_dir, receipt_tx_for_watcher)
        .expect("Failed to start intake folder watcher");

    tracing::info!("Watching intake folder: {}", intake_dir.display());

    // ── Tauri app ─────────────────────────────────────────────────────────────
    let state = AppState { db, attachments_dir, receipt_tx };

    tauri::Builder::default()
        // Mobile receipt intake: camera capture uses the WebView's native
        // <input type="file" capture="camera"> API; tauri-plugin-dialog provides
        // the gallery file picker on both desktop and mobile.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(Arc::new(Mutex::new(state)))
        .invoke_handler(tauri::generate_handler![
            commands::get_accounts,
            commands::create_transaction,
            commands::get_transactions,
            commands::get_profit_loss,
            commands::ingest_receipt,
            commands::get_pending_receipts,
            commands::approve_receipt,
            commands::reject_receipt,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
