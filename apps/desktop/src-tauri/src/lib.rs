#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::significant_drop_tightening,
    clippy::large_stack_frames,
    clippy::no_effect_underscore_binding,
    clippy::unused_self,
    clippy::wildcard_imports,
    clippy::single_match_else,
    clippy::field_reassign_with_default,
    clippy::unused_async,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_ref_mut,
    clippy::assigning_clones,
    clippy::implicit_hasher,
    clippy::large_enum_variant,
    clippy::struct_field_names,
    clippy::redundant_closure_for_method_calls,
    clippy::missing_const_for_thread_local,
    clippy::unnecessary_wraps,
    clippy::redundant_clone,
    clippy::format_push_string,
    clippy::bool_assert_comparison
)]
#![allow(unreachable_pub, dead_code)]

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
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
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
            commands::oauth_anthropic_start,
            commands::oauth_anthropic_complete,
            commands::oauth_openai_start,
            commands::oauth_openai_cancel,
            commands::oauth_logout,
            commands::run_case,
            commands::list_cases,
            commands::show_case,
            commands::submit_feedback,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Conclave");
}
