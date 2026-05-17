//! Ollama — local-only inference at `http://localhost:11434` by default.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "llama3.1:8b";

/// Local Ollama provider.
#[derive(Clone)]
pub struct OllamaProvider {
    base_url: String,
    default_model: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for OllamaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaProvider")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaProvider {
    /// Build a provider talking to the default localhost port.
    pub fn new() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: reqwest::Client::new(),
        }
    }

    /// Override the base URL.
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

    /// Ping `/api/tags` to confirm the server is alive.
    pub async fn ping(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn id(&self) -> &'static str {
        "ollama"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // Conservative default; depends on the loaded model.
            max_context_tokens: 8_192,
            supports_json_mode: true,
            supports_streaming: true,
            vision: false,
            scope: ProviderScope::General,
        }
    }

    fn requires_network(&self) -> bool {
        false
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let messages: Vec<OllamaMessage> = req
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                },
                content: m.content.clone(),
            })
            .collect();

        let body = OllamaRequest {
            model: if req.model.is_empty() {
                self.default_model.clone()
            } else {
                req.model.clone()
            },
            messages,
            stream: false,
            format: req.json_schema.as_ref().map(|_| "json"),
            options: OllamaOptions {
                temperature: req.temperature,
                num_predict: req.max_output_tokens,
            },
        };

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status().as_u16() == 404 {
            return Err(ProviderError::BadRequest(
                "ollama endpoint /api/chat not found — is the server running?".into(),
            ));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("{status}: {text}")));
        }

        let parsed: OllamaResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("ollama parse: {e}")))?;
        Ok(CompletionResponse {
            text: parsed.message.content,
            usage: Usage {
                input_tokens: parsed.prompt_eval_count.unwrap_or(0),
                output_tokens: parsed.eval_count.unwrap_or(0),
            },
            model: parsed.model,
            web_citations: Vec::new(),
        })
    }
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaResponseMessage,
    model: String,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}
