use std::sync::Arc;
use tokio::sync::Mutex;

mod commands;

pub struct AppState {
    pub db: aequi_storage::DbPool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app_dir = directories::ProjectDirs::from("com", "aequi", "Aequi")
        .expect("Failed to get app directory")
        .data_dir()
        .to_path_buf();

    std::fs::create_dir_all(&app_dir).expect("Failed to create app directory");

    let db_path = app_dir.join("ledger.db");
    let db = aequi_storage::create_db(&db_path)
        .await
        .expect("Failed to create database");

    aequi_storage::seed_default_accounts(&db)
        .await
        .expect("Failed to seed default accounts");

    let state = AppState { db };

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(state)))
        .invoke_handler(tauri::generate_handler![
            commands::get_accounts,
            commands::create_transaction,
            commands::get_transactions,
            commands::get_profit_loss,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
