//! Runtime registry of configured providers.
//!
//! Built from the on-disk config + keychain at startup so callers (verdict
//! pipeline, CLI subcommands) can look providers up by id.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::ProviderError;
use crate::{
    secrets, AnthropicOAuthProvider, AnthropicProvider, AppleIntelligenceProvider, LlmProvider,
    OllamaProvider, OpenAIOAuthProvider, OpenAiProvider, OpenRouterProvider,
};

/// API-key + local providers. Listed in the order the UI groups them:
/// API keys first, then on-device providers (Ollama and Apple
/// Intelligence) which are always available without credentials.
pub const KNOWN_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "openrouter",
    "ollama",
    "apple-intelligence",
];

/// OAuth (subscription-based) providers. Experimental.
pub const OAUTH_PROVIDERS: &[&str] = &["anthropic-oauth", "openai-oauth"];

/// Lookup-only collection of provider handles indexed by their stable id.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    inner: BTreeMap<&'static str, Arc<dyn LlmProvider>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&&str> = self.inner.keys().collect();
        f.debug_struct("ProviderRegistry")
            .field("providers", &ids)
            .finish()
    }
}

impl ProviderRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a registry that loads any keys present in the OS keychain for
    /// the well-known provider ids and registers a provider for each.
    /// Ollama is always registered; it self-reports availability at call time
    /// via [`OllamaProvider::ping`].
    pub fn from_keychain() -> Result<Self, ProviderError> {
        let mut me = Self::new();
        me.inner.insert("ollama", Arc::new(OllamaProvider::new()));
        // Apple Intelligence is registered unconditionally; the
        // provider itself reports `Availability::FrameworkUnavailable`
        // on hosts where the on-device model isn't reachable, so the
        // UI can render a "not supported" card without a separate
        // platform check here.
        me.inner.insert(
            "apple-intelligence",
            Arc::new(AppleIntelligenceProvider::new()),
        );
        if let Some(key) = secrets::load("anthropic")? {
            me.inner
                .insert("anthropic", Arc::new(AnthropicProvider::new(key)));
        }
        if let Some(key) = secrets::load("openai")? {
            me.inner
                .insert("openai", Arc::new(OpenAiProvider::new(key)));
        }
        if let Some(key) = secrets::load("openrouter")? {
            me.inner
                .insert("openrouter", Arc::new(OpenRouterProvider::new(key)));
        }
        if let Ok(p) = AnthropicOAuthProvider::from_default_location() {
            me.inner.insert("anthropic-oauth", Arc::new(p));
        }
        if let Ok(p) = OpenAIOAuthProvider::from_default_location() {
            me.inner.insert("openai-oauth", Arc::new(p));
        }
        Ok(me)
    }

    /// Register a provider implementation under its [`LlmProvider::id`] key.
    pub fn register(&mut self, provider: Arc<dyn LlmProvider>) {
        self.inner.insert(provider.id(), provider);
    }

    /// Look up a provider by its id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn LlmProvider>> {
        self.inner.get(id).cloned()
    }

    /// Iterate over every registered provider.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Arc<dyn LlmProvider>)> {
        self.inner.iter().map(|(k, v)| (*k, v))
    }

    /// True when at least one provider is registered.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
