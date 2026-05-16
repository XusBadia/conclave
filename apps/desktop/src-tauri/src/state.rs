//! Shared runtime state held by the Tauri app.

use std::sync::Mutex;

use conclave_core::{paths::Paths, Config};

/// One per app instance.
pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
}

impl AppState {
    pub fn new(paths: Paths, config: Config) -> Self {
        Self {
            paths,
            config: Mutex::new(config),
        }
    }
}
