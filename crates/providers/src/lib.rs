//! LLM provider abstraction used by the Conclave virtual committee.
//!
//! Phase 2 introduces a real implementation set: `Anthropic`, `OpenAI`,
//! `OpenRouter` and `Ollama`. Every concrete provider implements the
//! [`LlmProvider`] trait; secrets live in the OS keychain via the
//! [`secrets`] module; the [`ProviderRegistry`] composes them at runtime
//! for callers (verdict pipeline, CLI subcommands).
//!
//! ## Privacy invariants
//!
//! - No provider implementation logs the user message body or the response
//!   text. They log only model ids, latency, status codes, and token usage.
//! - Secrets never touch on-disk config files. They live in the OS keychain
//!   exclusively, under the `Conclave` service.

#![allow(clippy::similar_names)]

use async_trait::async_trait;

mod anthropic_api;
mod anthropic_oauth;
mod error;
mod mock;
mod ollama_local;
mod openai_api;
mod openai_oauth;
mod openrouter_api;
mod registry;
pub mod secrets;
mod types;

pub use anthropic_api::AnthropicProvider;
pub use anthropic_oauth::AnthropicOAuthProvider;
pub use error::ProviderError;
pub use mock::MockProvider;
pub use ollama_local::OllamaProvider;
pub use openai_api::OpenAiProvider;
pub use openai_oauth::OpenAIOAuthProvider;
pub use openrouter_api::OpenRouterProvider;
pub use registry::{ProviderRegistry, KNOWN_PROVIDERS, OAUTH_PROVIDERS};
pub use types::{
    CompletionRequest, CompletionResponse, Message, MessageRole, ProviderCapabilities, Usage,
};

/// Anything that can take a structured request and return generated text.
#[async_trait]
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Stable identifier (`anthropic`, `openai`, `openrouter`, `ollama`,
    /// `mock`). Used as the keychain account suffix and as the on-disk
    /// routing key.
    fn id(&self) -> &'static str;

    /// Capability flags so the caller can fail-fast on context overflow or
    /// reject JSON-mode requests against a provider that can't honour them.
    fn capabilities(&self) -> ProviderCapabilities;

    /// `true` when the provider needs an internet connection at call time.
    fn requires_network(&self) -> bool;

    /// Run a one-shot completion.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_round_trip() {
        let p = MockProvider::with_response("hola");
        let resp = p.complete(CompletionRequest::user("hi")).await.unwrap();
        assert_eq!(resp.text, "hola");
        assert_eq!(p.captured_requests().len(), 1);
    }

    #[test]
    fn registry_is_empty_by_default() {
        let r = ProviderRegistry::new();
        assert!(r.is_empty());
    }
}
