use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::{Mutex, mpsc};

pub mod commands;

/// OCR backend: Tesseract when built with the `tesseract` feature, otherwise
/// a stub that recognizes no text (receipts are stored for manual review).
#[cfg(feature = "tesseract")]
pub type Recognizer = aequi_ocr::recognizer::tesseract_backend::TesseractRecognizer;
#[cfg(not(feature = "tesseract"))]
pub type Recognizer = aequi_ocr::MockRecognizer;

fn new_recognizer() -> Recognizer {
    #[cfg(feature = "tesseract")]
    {
        Recognizer::new(None, "eng")
    }
    #[cfg(not(feature = "tesseract"))]
    {
        Recognizer::new("")
    }
}

pub struct AppState {
    pub db: aequi_storage::DbPool,
    pub db_path: PathBuf,
    /// The app data directory holding `ledger.db`, `attachments/`, `intake/`
    /// and `captures/`.
    pub data_dir: PathBuf,
    pub attachments_dir: PathBuf,
    /// Where the frontend writes camera captures before calling `ingest_receipt`.
    pub captures_dir: PathBuf,
    pub pipeline: Arc<aequi_ocr::ReceiptPipeline<Recognizer>>,
    /// Kept alive for the app's lifetime; dropping it stops the watcher.
    #[cfg(desktop)]
    pub _intake_watcher: Option<aequi_ocr::pipeline::IntakeWatcher>,
}

/// Build the shared Tauri app (used by both desktop main.rs and mobile lib entry).
///
/// The `aequi-mcp` server binary is bundled alongside the app (`externalBin`)
/// but not launched by it: MCP clients start it themselves over stdio.
pub fn build_app() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| format!("Failed to create data directory: {e}"))?;

            let db_path = data_dir.join("ledger.db");
            let attachments_dir = data_dir.join("attachments");
            let intake_dir = data_dir.join("intake");
            let captures_dir = data_dir.join("captures");
            for dir in [&attachments_dir, &intake_dir, &captures_dir] {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
            }

            let rt = tauri::async_runtime::handle();

            let db = rt
                .block_on(aequi_storage::create_db(&db_path))
                .map_err(|e| format!("Failed to create database: {e}"))?;

            rt.block_on(aequi_storage::seed_default_accounts(&db))
                .map_err(|e| format!("Failed to seed default accounts: {e}"))?;

            let pipeline = Arc::new(aequi_ocr::ReceiptPipeline::new(
                new_recognizer(),
                attachments_dir.clone(),
            ));

            // Receipt intake pipeline: the watcher sends each completely
            // written file once; it is deleted after it has been stored (the
            // pipeline keeps a copy under attachments/), so it is never
            // processed twice. Files that fail stay put and are retried on the
            // next start.
            let (receipt_tx, mut receipt_rx) = mpsc::channel::<PathBuf>(16);

            let db_for_pipeline = db.clone();
            let pipeline_for_intake = Arc::clone(&pipeline);

            tauri::async_runtime::spawn(async move {
                while let Some(path) = receipt_rx.recv().await {
                    if !path.is_file() {
                        // Already processed and removed.
                        continue;
                    }
                    tracing::info!("Processing receipt: {}", path.display());
                    match commands::store_receipt(&db_for_pipeline, &pipeline_for_intake, &path)
                        .await
                    {
                        Ok(id) => {
                            tracing::info!("Receipt stored: id {id}");
                            if let Err(e) = tokio::fs::remove_file(&path).await {
                                tracing::warn!(
                                    "Failed to remove processed intake file {}: {e}",
                                    path.display()
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Receipt intake failed for {}: {} ({})",
                                path.display(),
                                e.message,
                                e.code
                            );
                        }
                    }
                }
            });

            // Watch folder (desktop only — on mobile, files come via camera capture)
            #[cfg(desktop)]
            let intake_watcher =
                match aequi_ocr::pipeline::spawn_intake_watcher(&intake_dir, receipt_tx) {
                    Ok(watcher) => {
                        tracing::info!("Watching intake folder: {}", intake_dir.display());
                        Some(watcher)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start intake folder watcher: {e}");
                        None
                    }
                };
            #[cfg(not(desktop))]
            drop(receipt_tx);

            let state = AppState {
                db,
                db_path,
                data_dir,
                attachments_dir,
                captures_dir,
                pipeline,
                #[cfg(desktop)]
                _intake_watcher: intake_watcher,
            };
            app.manage(Arc::new(Mutex::new(state)));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_accounts,
            commands::create_transaction,
            commands::get_transactions,
            commands::get_profit_loss,
            commands::ingest_receipt,
            commands::get_pending_receipts,
            commands::approve_receipt,
            commands::reject_receipt,
            commands::estimate_quarterly_tax,
            commands::get_schedule_c_preview,
            commands::get_contacts,
            commands::create_contact,
            commands::get_invoices,
            commands::create_invoice,
            commands::get_invoice_aging,
            commands::record_invoice_payment,
            commands::get_1099_summary,
            commands::send_invoice,
            commands::export_beancount,
            commands::export_qif,
            commands::get_setting,
            commands::set_setting,
            commands::get_audit_log,
            commands::get_schema_versions,
            commands::check_for_updates,
            commands::check_overdue_invoices,
            commands::get_dashboard_summary,
            commands::update_contact,
        ])
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
fn main() {
    tracing_subscriber::fmt::init();

    build_app()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
