//! Anthropic Messages API (`api.anthropic.com/v1/messages`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, Usage,
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
            client: reqwest::Client::new(),
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
                MessageRole::User => convo.push(AnthropicMessage::new("user", m.content)),
                MessageRole::Assistant => {
                    convo.push(AnthropicMessage::new("assistant", m.content));
                }
            }
        }

        if convo.is_empty() {
            return Err(ProviderError::BadRequest(
                "anthropic requires at least one user/assistant message".into(),
            ));
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
    content: String,
}

impl AnthropicMessage {
    const fn new(role: &'static str, content: String) -> Self {
        Self { role, content }
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
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async { p.complete(req).await }).unwrap_err();
        matches!(err, ProviderError::BadRequest(_));
    }
}
