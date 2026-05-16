//! Shared runtime state held by the Tauri app.

use std::sync::Mutex;

use conclave_core::{paths::Paths, Config};
use conclave_providers::AnthropicLoginFlow;

/// One per app instance.
pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
    /// In-flight Anthropic OAuth login (PKCE pair + state) waiting for the
    /// user to paste their code. At most one flow may be active at a time.
    pub anthropic_login: Mutex<Option<AnthropicLoginFlow>>,
}

impl AppState {
    pub fn new(paths: Paths, config: Config) -> Self {
        Self {
            paths,
            config: Mutex::new(config),
            anthropic_login: Mutex::new(None),
        }
    }
}
