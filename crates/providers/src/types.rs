//! Shared request / response / capability types for every provider impl.

use serde::{Deserialize, Serialize};

/// Capability flags advertised by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// Approximate maximum context window in tokens.
    pub max_context_tokens: u32,
    /// Provider can be asked for valid JSON output.
    pub supports_json_mode: bool,
    /// Provider exposes a streaming endpoint.
    pub supports_streaming: bool,
    /// Provider accepts image input.
    pub vision: bool,
}

/// Role of a chat message inside a [`CompletionRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    /// Top-level instruction. Anthropic flattens these into the `system` field.
    System,
    /// Human / clinician input.
    User,
    /// Prior model output, for multi-turn conversations.
    Assistant,
}

/// A single chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message author.
    pub role: MessageRole,
    /// Verbatim text content. Multi-modal payloads land later.
    pub content: String,
}

impl Message {
    /// Build a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }
    /// Build a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }
    /// Build an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

/// Inference request sent to a provider.
#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    /// Model id understood by the provider. Empty string → provider default.
    pub model: String,
    /// Ordered conversation. System messages may appear anywhere; each
    /// provider impl rolls them up as appropriate.
    pub messages: Vec<Message>,
    /// Output cap. `None` lets the provider pick a sensible default.
    pub max_output_tokens: Option<u32>,
    /// Sampling temperature. `None` lets the provider pick its default.
    pub temperature: Option<f32>,
    /// When set, instructs the provider to constrain its output to a JSON
    /// shape. Not every provider can guarantee schema compliance; flat
    /// JSON-mode is the minimum bar.
    pub json_schema: Option<serde_json::Value>,
    /// If `true`, providers that support live web search should enable it
    /// (e.g. Codex's `web_search_preview` tool). Providers without web
    /// support silently ignore the flag and return empty `web_citations`.
    pub allow_web_search: bool,
}

impl CompletionRequest {
    /// Build a minimal user-only request with the provider's default model.
    pub fn user(prompt: impl Into<String>) -> Self {
        Self {
            model: String::new(),
            messages: vec![Message::user(prompt)],
            max_output_tokens: None,
            temperature: None,
            json_schema: None,
            allow_web_search: false,
        }
    }
}

/// A web page the model consulted to answer the question, surfaced so the
/// UI can show "answer used the web" disclosure with clickable links.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WebCitation {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub snippet: String,
}

/// Successful completion result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Generated text (already concatenated across provider chunks).
    pub text: String,
    /// Token accounting reported by the provider.
    pub usage: Usage,
    /// Model id that actually served the request, as echoed by the provider.
    pub model: String,
    /// URLs the model cited from a live web search, in order of first
    /// appearance in the answer. Empty when the provider doesn't run a
    /// web search or the model didn't trigger one.
    #[serde(default)]
    pub web_citations: Vec<WebCitation>,
}

/// Token usage reported by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the prompt.
    pub input_tokens: u32,
    /// Tokens produced by the model.
    pub output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_set_role() {
        assert_eq!(Message::system("a").role, MessageRole::System);
        assert_eq!(Message::user("a").role, MessageRole::User);
        assert_eq!(Message::assistant("a").role, MessageRole::Assistant);
    }

    #[test]
    fn role_serialises_lowercase() {
        let json = serde_json::to_string(&Message::assistant("hi")).unwrap();
        assert!(json.contains("\"assistant\""));
    }
}
