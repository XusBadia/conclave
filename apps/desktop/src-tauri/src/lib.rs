#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::needless_pass_by_value
)]

//! Conclave desktop — Tauri 2 entry point.
//!
//! The frontend (React + Vite) calls into `commands` to drive the core
//! Conclave crates. Every Tauri command is a thin wrapper over a
//! CLI-equivalent core function so the UI never holds business logic.

mod commands;
mod state;

use tauri::Manager;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            let paths = conclave_core::paths::Paths::resolve()
                .map_err(|e| format!("could not resolve app dirs: {e}"))?;
            paths
                .ensure_exists()
                .map_err(|e| format!("ensure dirs: {e}"))?;
            let config = conclave_core::Config::load(paths.config_file())
                .map_err(|e| format!("load config: {e}"))?;
            app.manage(state::AppState::new(paths, config));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::onboarding_status,
            commands::accept_disclaimer,
            commands::list_workspaces,
            commands::create_workspace,
            commands::switch_workspace,
            commands::active_workspace,
            commands::delete_workspace,
            commands::list_documents,
            commands::show_document,
            commands::remove_document,
            commands::ingest_path,
            commands::search_workspace,
            commands::deident_text,
            commands::list_providers,
            commands::set_provider_key,
            commands::test_provider,
            commands::remove_provider_key,
            commands::run_case,
            commands::list_cases,
            commands::show_case,
            commands::submit_feedback,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Conclave");
}
