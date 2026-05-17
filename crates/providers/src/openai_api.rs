//! `OpenAI` chat-completions API. The wire format is also used by
//! `OpenRouter`, so both providers share the helpers in this module.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_MODEL: &str = "gpt-5";

/// API-key `OpenAI` provider.
#[derive(Clone)]
pub struct OpenAiProvider {
    api_key: String,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl OpenAiProvider {
    /// Build a provider with the given API key.
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
impl LlmProvider for OpenAiProvider {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 128_000,
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
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut messages = build_chat_messages(req.messages.as_slice());
        attach_images_to_last_user(&mut messages, &req.images);
        chat_completions_call(
            &self.client,
            &url,
            ChatHeaders {
                bearer: Some(&self.api_key),
                extra: &[],
            },
            ChatBody {
                model: if req.model.is_empty() {
                    self.default_model.clone()
                } else {
                    req.model.clone()
                },
                messages,
                max_tokens: req.max_output_tokens,
                temperature: req.temperature,
                response_format: req.json_schema.as_ref().map(|_| ResponseFormat {
                    kind: "json_object",
                }),
            },
        )
        .await
    }
}

/// Build the `messages` field accepted by chat-completions endpoints.
pub(crate) fn build_chat_messages(messages: &[crate::types::Message]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| ChatMessage {
            role: match m.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            },
            content: ChatContentField::Text(m.content.clone()),
        })
        .collect()
}

/// Attach vision images to the last `user` message, converting its content
/// from a plain string into an array of content blocks if needed. No-op
/// when `images` is empty.
pub(crate) fn attach_images_to_last_user(
    messages: &mut Vec<ChatMessage>,
    images: &[crate::types::ImageInput],
) {
    if images.is_empty() {
        return;
    }
    let last_user_idx = match messages.iter().rposition(|m| m.role == "user") {
        Some(i) => i,
        None => {
            messages.push(ChatMessage {
                role: "user",
                content: ChatContentField::Blocks(Vec::new()),
            });
            messages.len() - 1
        }
    };
    if let ChatContentField::Text(t) = &messages[last_user_idx].content {
        let initial = if t.is_empty() {
            Vec::new()
        } else {
            vec![ChatBlock::Text { text: t.clone() }]
        };
        messages[last_user_idx].content = ChatContentField::Blocks(initial);
    }
    if let ChatContentField::Blocks(blocks) = &mut messages[last_user_idx].content {
        for img in images {
            blocks.push(ChatBlock::ImageUrl {
                image_url: ChatImageUrl {
                    url: format!("data:{};base64,{}", img.media_type, img.base64_data),
                },
            });
        }
    }
}

/// HTTP headers passed to a chat-completions call.
pub(crate) struct ChatHeaders<'a> {
    pub bearer: Option<&'a str>,
    pub extra: &'a [(&'a str, &'a str)],
}

/// Body of a chat-completions request.
#[derive(Serialize)]
pub(crate) struct ChatBody {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
pub(crate) struct ChatMessage {
    pub role: &'static str,
    pub content: ChatContentField,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum ChatContentField {
    Text(String),
    Blocks(Vec<ChatBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ChatBlock {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Serialize)]
pub(crate) struct ChatImageUrl {
    pub url: String,
}

#[derive(Serialize)]
pub(crate) struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    model: String,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// Shared chat-completions HTTP call used by `OpenAI` and `OpenRouter`.
pub(crate) async fn chat_completions_call(
    client: &reqwest::Client,
    url: &str,
    headers: ChatHeaders<'_>,
    body: ChatBody,
) -> Result<CompletionResponse, ProviderError> {
    let mut builder = client.post(url).json(&body);
    if let Some(token) = headers.bearer {
        builder = builder.bearer_auth(token);
    }
    for (k, v) in headers.extra {
        builder = builder.header(*k, *v);
    }

    let resp = builder
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
        if body_text.contains("context_length") || body_text.contains("maximum context") {
            return Err(ProviderError::ContextOverflow);
        }
        return Err(ProviderError::BadRequest(format!("{status}: {body_text}")));
    }

    let parsed: ChatResponse = resp
        .json()
        .await
        .map_err(|e| ProviderError::Other(format!("openai parse: {e}")))?;

    let text = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default();
    let usage = parsed.usage.unwrap_or_default();

    Ok(CompletionResponse {
        text,
        usage: Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        },
        model: parsed.model,
        web_citations: Vec::new(),
    })
}
