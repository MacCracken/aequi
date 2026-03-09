#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tracing_subscriber::fmt::init();

    aequi::build_app()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
