//! LLM provider abstraction used by the Conclave virtual committee.
//!
//! Phase 0 only defines the trait surface and the shared message/role types.
//! Concrete implementations (`Anthropic`, `OpenAI`, local `llama.cpp`, etc.)
//! are introduced in Phase 2.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use conclave_core::Result;

/// Role of a chat message in a provider conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Instruction-level system message.
    System,
    /// Human / clinician input.
    User,
    /// Model output.
    Assistant,
}

/// A single message inside a provider request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Role of the message author.
    pub role: Role,
    /// Verbatim content of the message.
    pub content: String,
}

impl Message {
    /// Build a system-role message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    /// Build a user-role message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    /// Build an assistant-role message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Generation request sent to a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationRequest {
    /// Ordered conversation history.
    pub messages: Vec<Message>,
    /// Sampling temperature, when supported by the underlying model.
    pub temperature: Option<f32>,
    /// Maximum number of output tokens, when supported.
    pub max_output_tokens: Option<u32>,
}

/// Generation response returned by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationResponse {
    /// The assistant message produced by the provider.
    pub message: Message,
    /// Token accounting, when reported.
    pub usage: Option<Usage>,
    /// Reason the provider stopped generating, when reported.
    pub stop_reason: Option<String>,
}

/// Token accounting reported by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the prompt.
    pub input_tokens: u32,
    /// Tokens produced by the model.
    pub output_tokens: u32,
}

/// Capability flags advertised by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Identifier the user references in config (`anthropic`, `openai`, ...).
    pub id: String,
    /// Human-readable provider name.
    pub display_name: String,
    /// Whether the provider streams partial output.
    pub supports_streaming: bool,
    /// Whether the provider can call tools.
    pub supports_tools: bool,
}

/// Trait every concrete LLM provider implements.
#[async_trait]
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Stable, machine-readable identifier (matches `Capabilities::id`).
    fn id(&self) -> &'static str;

    /// Capabilities advertised by this provider.
    fn capabilities(&self) -> Capabilities;

    /// Run a single-shot generation request.
    async fn generate(&self, request: GenerationRequest) -> Result<GenerationResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_set_role() {
        assert_eq!(Message::system("a").role, Role::System);
        assert_eq!(Message::user("a").role, Role::User);
        assert_eq!(Message::assistant("a").role, Role::Assistant);
    }

    #[test]
    fn role_serializes_lowercase() {
        let raw = toml::to_string(&Wrap {
            role: Role::Assistant,
        })
        .unwrap();
        assert!(raw.contains("role = \"assistant\""));
    }

    #[derive(Serialize)]
    struct Wrap {
        role: Role,
    }
}
