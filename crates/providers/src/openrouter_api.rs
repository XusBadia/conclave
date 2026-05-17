//! `OpenRouter` — `OpenAI`-compatible chat completions that fan out to
//! many models behind a single API key.

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::openai_api::{
    attach_images_to_last_user, build_chat_messages, chat_completions_call, ChatBody, ChatHeaders,
    ResponseFormat,
};
use crate::types::{CompletionRequest, CompletionResponse, ProviderCapabilities};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api";

/// API-key `OpenRouter` provider.
#[derive(Clone)]
pub struct OpenRouterProvider {
    api_key: String,
    base_url: String,
    default_model: Option<String>,
    client: reqwest::Client,
}

impl std::fmt::Debug for OpenRouterProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterProvider")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl OpenRouterProvider {
    /// Build a provider with the given API key. No default model — the
    /// caller must pass one (`OpenRouter` routes by model id).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: None,
            client: reqwest::Client::new(),
        }
    }

    /// Override the API base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Set the default model id.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(model.into());
        self
    }
}

#[async_trait]
impl LlmProvider for OpenRouterProvider {
    fn id(&self) -> &'static str {
        "openrouter"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 128_000,
            supports_json_mode: true,
            supports_streaming: true,
            vision: false,
        }
    }

    fn requires_network(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let model = if req.model.is_empty() {
            self.default_model.clone().ok_or_else(|| {
                ProviderError::BadRequest("openrouter requires an explicit model id".into())
            })?
        } else {
            req.model.clone()
        };

        let url = format!("{}/v1/chat/completions", self.base_url);
        chat_completions_call(
            &self.client,
            &url,
            ChatHeaders {
                bearer: Some(&self.api_key),
                extra: &[
                    ("HTTP-Referer", "https://github.com/XusBadia/conclave"),
                    ("X-Title", "Conclave"),
                ],
            },
            ChatBody {
                model,
                messages: {
                    let mut m = build_chat_messages(req.messages.as_slice());
                    attach_images_to_last_user(&mut m, &req.images);
                    m
                },
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
