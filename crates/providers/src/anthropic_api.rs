//! Anthropic Messages API (`api.anthropic.com/v1/messages`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-sonnet-4-6-20250929";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// API-key Anthropic provider.
#[derive(Clone)]
pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl AnthropicProvider {
    /// Build a provider with the given API key, default base url, default
    /// model.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: crate::cli_local::http_client(),
        }
    }

    /// Override the API base URL (useful for testing against `wiremock`).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Override the default model id.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 200_000,
            supports_json_mode: true,
            supports_streaming: true,
            vision: true,
            scope: ProviderScope::General,
        }
    }

    fn requires_network(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        // Anthropic takes system messages out of the conversation and into a
        // dedicated `system` field. We concatenate multiple system messages.
        let mut system_parts = Vec::new();
        let mut convo: Vec<AnthropicMessage> = Vec::new();
        for m in req.messages {
            match m.role {
                MessageRole::System => system_parts.push(m.content),
                MessageRole::User => convo.push(AnthropicMessage::text("user", m.content)),
                MessageRole::Assistant => {
                    convo.push(AnthropicMessage::text("assistant", m.content));
                }
            }
        }

        if convo.is_empty() {
            return Err(ProviderError::BadRequest(
                "anthropic requires at least one user/assistant message".into(),
            ));
        }

        // Attach images to the LAST user message in the conversation. If
        // the last message is from the assistant, fall back to appending a
        // new user message containing just the images — Anthropic requires
        // images to live on a user turn.
        if !req.images.is_empty() {
            let last_user_idx = convo
                .iter()
                .rposition(|m| m.role == "user")
                .unwrap_or(convo.len());
            if last_user_idx == convo.len() {
                convo.push(AnthropicMessage::blocks("user", Vec::new()));
            }
            convo[last_user_idx].promote_to_blocks();
            for img in &req.images {
                convo[last_user_idx].push_block(AnthropicBlock::Image {
                    source: AnthropicImageSource {
                        kind: "base64",
                        media_type: img.media_type.clone(),
                        data: img.base64_data.clone(),
                    },
                });
            }
        }

        let body = AnthropicRequest {
            model: if req.model.is_empty() {
                self.default_model.clone()
            } else {
                req.model.clone()
            },
            messages: convo,
            system: if system_parts.is_empty() {
                None
            } else {
                Some(system_parts.join("\n\n"))
            },
            max_tokens: req.max_output_tokens.unwrap_or(4096),
            temperature: req.temperature,
        };

        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
            return Err(ProviderError::Auth);
        }
        if resp.status().as_u16() == 429 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok());
            return Err(ProviderError::RateLimit {
                retry_after_secs: retry,
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if body_text.contains("context") && body_text.contains("length") {
                return Err(ProviderError::ContextOverflow);
            }
            return Err(ProviderError::BadRequest(format!("{status}: {body_text}")));
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("anthropic parse: {e}")))?;

        let text: String = parsed
            .content
            .iter()
            .filter(|c| c.kind.as_deref() == Some("text"))
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        Ok(CompletionResponse {
            text,
            usage: Usage {
                input_tokens: parsed.usage.input_tokens,
                output_tokens: parsed.usage.output_tokens,
            },
            model: parsed.model,
            web_citations: Vec::new(),
        })
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: AnthropicContentField,
}

#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicContentField {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text { text: String },
    Image { source: AnthropicImageSource },
}

#[derive(Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: String,
    data: String,
}

impl AnthropicMessage {
    fn text(role: &'static str, content: String) -> Self {
        Self {
            role,
            content: AnthropicContentField::Text(content),
        }
    }

    fn blocks(role: &'static str, blocks: Vec<AnthropicBlock>) -> Self {
        Self {
            role,
            content: AnthropicContentField::Blocks(blocks),
        }
    }

    /// Convert a text-only message into a blocks message so we can append
    /// image blocks to it. Idempotent if the message is already blocks.
    fn promote_to_blocks(&mut self) {
        if let AnthropicContentField::Text(t) = &self.content {
            let initial = if t.is_empty() {
                Vec::new()
            } else {
                vec![AnthropicBlock::Text { text: t.clone() }]
            };
            self.content = AnthropicContentField::Blocks(initial);
        }
    }

    fn push_block(&mut self, block: AnthropicBlock) {
        if let AnthropicContentField::Blocks(blocks) = &mut self.content {
            blocks.push(block);
        }
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn id_and_capabilities() {
        let p = AnthropicProvider::new("test-key");
        assert_eq!(p.id(), "anthropic");
        assert!(p.requires_network());
        let caps = p.capabilities();
        assert!(caps.max_context_tokens >= 100_000);
        assert!(caps.supports_json_mode);
    }

    #[test]
    fn empty_convo_is_rejected() {
        let p = AnthropicProvider::new("k");
        let req = CompletionRequest {
            model: String::new(),
            messages: vec![Message::system("only system")],
            max_output_tokens: None,
            temperature: None,
            json_schema: None,
            allow_web_search: false,
            images: Vec::new(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async { p.complete(req).await }).unwrap_err();
        matches!(err, ProviderError::BadRequest(_));
    }
}
