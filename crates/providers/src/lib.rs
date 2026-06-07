#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::needless_pass_by_value,
    clippy::struct_field_names,
    clippy::items_after_statements,
    clippy::significant_drop_tightening,
    clippy::single_match_else,
    clippy::map_unwrap_or,
    clippy::option_if_let_else,
    clippy::redundant_clone,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports,
    clippy::missing_const_for_fn,
    clippy::assigning_clones,
    clippy::implicit_hasher,
    clippy::format_push_string,
    clippy::redundant_closure_for_method_calls,
    clippy::unnecessary_join,
    clippy::needless_collect,
    clippy::bool_assert_comparison,
    clippy::single_char_pattern,
    clippy::or_fun_call,
    clippy::option_map_unit_fn,
    clippy::needless_match,
    clippy::single_match,
    clippy::if_then_some_else_none,
    clippy::manual_let_else,
    unreachable_pub
)]

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

use async_trait::async_trait;

mod anthropic_api;
mod anthropic_oauth;
mod apple_intelligence;
mod claude_cli;
mod cli_local;
mod codex_cli;
mod error;
mod mock;
mod oauth_flow;
mod ollama_local;
mod openai_api;
mod openai_oauth;
mod openrouter_api;
mod registry;
pub mod secrets;
mod types;

pub use anthropic_api::AnthropicProvider;
pub use anthropic_oauth::AnthropicOAuthProvider;
pub use apple_intelligence::{
    AppleIntelligenceProvider, Availability as AppleIntelligenceAvailability,
    DEFAULT_MODEL_LABEL as APPLE_INTELLIGENCE_MODEL_LABEL,
};
pub use claude_cli::{
    ClaudeCliProvider, DEFAULT_MODEL as CLAUDE_CLI_DEFAULT_MODEL,
    PROVIDER_ID as CLAUDE_CLI_PROVIDER_ID,
};
pub use cli_local::ProbeDetails;
pub use codex_cli::{
    CodexCliProvider, DEFAULT_MODEL as CODEX_CLI_DEFAULT_MODEL,
    PROVIDER_ID as CODEX_CLI_PROVIDER_ID,
};
pub use error::ProviderError;
pub use mock::MockProvider;
pub use oauth_flow::{
    conclave_oauth_path, open_in_browser, persist_tokens, AnthropicLoginFlow, OAuthTokens,
    OpenAILoginFlow, Started,
};
pub use ollama_local::OllamaProvider;
pub use openai_api::OpenAiProvider;
pub use openai_oauth::OpenAIOAuthProvider;
pub use openrouter_api::OpenRouterProvider;
pub use registry::{ProviderRegistry, CLI_PROVIDERS, KNOWN_PROVIDERS, OAUTH_PROVIDERS};
pub use types::{
    CompletionRequest, CompletionResponse, ImageInput, Message, MessageRole, ProviderCapabilities,
    ProviderScope, Usage, WebCitation,
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
