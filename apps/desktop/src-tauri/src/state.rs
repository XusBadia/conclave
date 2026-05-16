//! Shared runtime state held by the Tauri app.

use std::sync::Mutex;

use conclave_core::{paths::Paths, Config};
use conclave_providers::AnthropicLoginFlow;
use tokio::task::AbortHandle;

/// One per app instance.
pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
    /// In-flight Anthropic OAuth login (PKCE pair + state) waiting for the
    /// user to paste their code. At most one flow may be active at a time.
    pub anthropic_login: Mutex<Option<AnthropicLoginFlow>>,
    /// In-flight OpenAI OAuth task. Holding the abort handle here lets the
    /// UI cancel a stuck flow, which drops the localhost:1455 listener so
    /// the next attempt can bind cleanly.
    pub openai_login: Mutex<Option<AbortHandle>>,
}

impl AppState {
    pub fn new(paths: Paths, config: Config) -> Self {
        Self {
            paths,
            config: Mutex::new(config),
            anthropic_login: Mutex::new(None),
            openai_login: Mutex::new(None),
        }
    }
}
