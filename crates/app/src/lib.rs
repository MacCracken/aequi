use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::{mpsc, Mutex};

pub mod commands;

pub struct AppState {
    pub db: aequi_storage::DbPool,
    pub attachments_dir: PathBuf,
    pub receipt_tx: mpsc::Sender<PathBuf>,
}

/// Build the shared Tauri app (used by both desktop main.rs and mobile lib entry).
pub fn build_app() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

            let db_path = data_dir.join("ledger.db");
            let attachments_dir = data_dir.join("attachments");
            let intake_dir = data_dir.join("intake");
            std::fs::create_dir_all(&attachments_dir)
                .expect("Failed to create attachments directory");
            std::fs::create_dir_all(&intake_dir).expect("Failed to create intake directory");

            let rt = tauri::async_runtime::handle();

            let db = rt
                .block_on(aequi_storage::create_db(&db_path))
                .expect("Failed to create database");

            rt.block_on(aequi_storage::seed_default_accounts(&db))
                .expect("Failed to seed default accounts");

            // Receipt intake pipeline
            let (receipt_tx, mut receipt_rx) = mpsc::channel::<PathBuf>(64);

            let db_for_pipeline = db.clone();
            let attachments_for_pipeline = attachments_dir.clone();

            tauri::async_runtime::spawn(async move {
                use aequi_ocr::{MockRecognizer, ReceiptPipeline};

                let pipeline =
                    ReceiptPipeline::new(MockRecognizer::new(""), attachments_for_pipeline);

                while let Some(path) = receipt_rx.recv().await {
                    tracing::info!("Processing receipt: {}", path.display());
                    match pipeline.process_file(&path).await {
                        Ok(result) => {
                            let e = &result.extracted;
                            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("bin");
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
                                e.payment_method
                                    .as_ref()
                                    .map(|f| f.value.to_string())
                                    .as_deref(),
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

            // Watch folder (desktop only — on mobile, files come via camera capture)
            #[cfg(desktop)]
            {
                let receipt_tx_for_watcher = receipt_tx.clone();
                let watcher =
                    aequi_ocr::pipeline::spawn_intake_watcher(&intake_dir, receipt_tx_for_watcher)
                        .expect("Failed to start intake folder watcher");
                // Leak the watcher so it lives for the app's lifetime.
                // Tauri doesn't provide a place to store arbitrary owned values.
                std::mem::forget(watcher);
                tracing::info!("Watching intake folder: {}", intake_dir.display());
            }

            let state = AppState {
                db,
                attachments_dir,
                receipt_tx,
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
            commands::export_beancount,
            commands::export_qif,
            commands::get_setting,
            commands::set_setting,
            commands::get_audit_log,
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
