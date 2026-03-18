use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::{mpsc, Mutex};

pub mod commands;

pub struct AppState {
    pub db: aequi_storage::DbPool,
    pub db_path: PathBuf,
    pub attachments_dir: PathBuf,
    pub receipt_tx: mpsc::Sender<PathBuf>,
    /// Kept alive for the app's lifetime; dropping it stops the watcher.
    #[cfg(desktop)]
    pub _intake_watcher: Option<Box<dyn std::any::Any + Send>>,
}

/// Spawn the MCP server as a sidecar process (desktop only).
///
/// The sidecar binary (`aequi-mcp`) communicates via stdio JSON-RPC 2.0.
/// We pass `AEQUI_DB_PATH` so it shares the same database as the Tauri app.
#[cfg(desktop)]
fn spawn_mcp_sidecar(app: &tauri::App, db_path: &std::path::Path) {
    use tauri_plugin_shell::ShellExt;

    let sidecar = match app
        .shell()
        .sidecar("binaries/aequi-mcp")
    {
        Ok(cmd) => cmd.env("AEQUI_DB_PATH", db_path.to_string_lossy().to_string()),
        Err(e) => {
            tracing::warn!("failed to create aequi-mcp sidecar command: {e}");
            return;
        }
    };

    match sidecar.spawn() {
        Ok((mut _rx, _child)) => {
            tracing::info!("aequi-mcp sidecar spawned (pid managed by Tauri)");
            // Log stderr output from the sidecar for debugging
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                while let Some(event) = _rx.recv().await {
                    match event {
                        CommandEvent::Stderr(line) => {
                            let msg = String::from_utf8_lossy(&line);
                            tracing::debug!(target: "aequi-mcp", "{}", msg.trim());
                        }
                        CommandEvent::Terminated(payload) => {
                            tracing::info!(
                                "aequi-mcp sidecar exited (code={:?}, signal={:?})",
                                payload.code,
                                payload.signal,
                            );
                            break;
                        }
                        CommandEvent::Error(err) => {
                            tracing::warn!("aequi-mcp sidecar error: {err}");
                        }
                        _ => {}
                    }
                }
            });
        }
        Err(e) => {
            tracing::warn!("failed to spawn aequi-mcp sidecar: {e}");
        }
    }
}

/// Build the shared Tauri app (used by both desktop main.rs and mobile lib entry).
pub fn build_app() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| format!("Failed to create data directory: {e}"))?;

            let db_path = data_dir.join("ledger.db");
            let attachments_dir = data_dir.join("attachments");
            let intake_dir = data_dir.join("intake");
            std::fs::create_dir_all(&attachments_dir)
                .map_err(|e| format!("Failed to create attachments directory: {e}"))?;
            std::fs::create_dir_all(&intake_dir)
                .map_err(|e| format!("Failed to create intake directory: {e}"))?;

            let rt = tauri::async_runtime::handle();

            let db = rt
                .block_on(aequi_storage::create_db(&db_path))
                .map_err(|e| format!("Failed to create database: {e}"))?;

            rt.block_on(aequi_storage::seed_default_accounts(&db))
                .map_err(|e| format!("Failed to seed default accounts: {e}"))?;

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
            let intake_watcher = {
                let receipt_tx_for_watcher = receipt_tx.clone();
                match aequi_ocr::pipeline::spawn_intake_watcher(&intake_dir, receipt_tx_for_watcher) {
                    Ok(watcher) => {
                        tracing::info!("Watching intake folder: {}", intake_dir.display());
                        Some(Box::new(watcher) as Box<dyn std::any::Any + Send>)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start intake folder watcher: {e}");
                        None
                    }
                }
            };

            // Spawn MCP sidecar (desktop only)
            #[cfg(desktop)]
            spawn_mcp_sidecar(app, &db_path);

            let state = AppState {
                db,
                db_path,
                attachments_dir,
                receipt_tx,
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
            commands::create_backup,
            commands::restore_backup,
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
