//! OpenAI ChatGPT subscription provider — reads the OAuth credentials
//! dropped on disk by the official OpenAI Codex CLI (`codex login`) and
//! uses them to call the ChatGPT-backed Responses API.
//!
//! **Experimental.** OpenAI does not publicly document the endpoints used
//! by Codex; the schema may change without notice. Users must accept the
//! ChatGPT Terms of Service to use this path. For a stable, supported
//! integration use [`crate::OpenAiProvider`] with a regular API key.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, Usage,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_MODEL: &str = "gpt-5";
const USER_AGENT: &str = "codex_cli_rs/Conclave";
const ORIGINATOR: &str = "codex_cli_rs";

/// ChatGPT-subscription provider that piggybacks on the Codex CLI's
/// credentials file.
pub struct OpenAIOAuthProvider {
    credentials_path: PathBuf,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
    cached: Mutex<Option<OAuthTokens>>,
}

impl std::fmt::Debug for OpenAIOAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIOAuthProvider")
            .field("credentials_path", &self.credentials_path)
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl OpenAIOAuthProvider {
    /// Build a provider that reads `~/.codex/auth.json` (or whatever
    /// `CONCLAVE_CODEX_CREDENTIALS` env var points to).
    pub fn from_default_location() -> Result<Self, ProviderError> {
        let path = default_credentials_path()?;
        Self::from_path(path)
    }

    /// Build with an explicit credentials file path.
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let path = path.into();
        let tokens = load_credentials(&path)?;
        Ok(Self {
            credentials_path: path,
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: reqwest::Client::new(),
            cached: Mutex::new(Some(tokens)),
        })
    }

    /// Override the API base URL (testing).
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

    /// Account id parsed from the credentials file, if present.
    pub fn account_id(&self) -> Option<String> {
        self.cached
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|t| t.account_id.clone()))
    }

    fn current_token(&self) -> Result<String, ProviderError> {
        let guard = self
            .cached
            .lock()
            .map_err(|_| ProviderError::Other("oauth cache poisoned".into()))?;
        if let Some(t) = guard.as_ref() {
            return Ok(t.access_token.clone());
        }
        drop(guard);
        let fresh = load_credentials(&self.credentials_path)?;
        let token = fresh.access_token.clone();
        if let Ok(mut g) = self.cached.lock() {
            *g = Some(fresh);
        }
        Ok(token)
    }
}

#[async_trait]
impl LlmProvider for OpenAIOAuthProvider {
    fn id(&self) -> &'static str {
        "openai-oauth"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 128_000,
            supports_json_mode: false, // Responses API has its own format.
            supports_streaming: true,
            vision: false,
        }
    }

    fn requires_network(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let token = self.current_token()?;
        let model = if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        };

        // Build the input array the Responses API expects.
        let input: Vec<ResponseInput> = req
            .messages
            .into_iter()
            .map(|m| ResponseInput {
                kind: "message",
                role: match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                },
                content: vec![ResponseContent {
                    kind: match m.role {
                        MessageRole::Assistant => "output_text".to_owned(),
                        _ => "input_text".to_owned(),
                    },
                    text: m.content,
                }],
            })
            .collect();
        if input.is_empty() {
            return Err(ProviderError::BadRequest(
                "openai-oauth requires at least one message".into(),
            ));
        }

        let body = ResponseRequest {
            model,
            input,
            store: false,
            max_output_tokens: req.max_output_tokens,
            temperature: req.temperature,
        };

        let session_id = uuid_v4();
        let url = format!("{}/codex/responses", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("OpenAI-Beta", "responses=v1")
            .header("originator", ORIGINATOR)
            .header("session_id", session_id)
            .header("user-agent", USER_AGENT)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
            return Err(ProviderError::Auth);
        }
        if resp.status().as_u16() == 429 {
            return Err(ProviderError::RateLimit {
                retry_after_secs: None,
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::BadRequest(format!("{status}: {body_text}")));
        }

        let parsed: ResponseEnvelope = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("openai-oauth parse: {e}")))?;

        let text = parsed
            .output
            .iter()
            .flat_map(|o| o.content.as_deref().unwrap_or(&[]))
            .filter(|c| c.kind == "output_text" || c.kind == "text")
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("");
        let usage = parsed.usage.unwrap_or_default();
        Ok(CompletionResponse {
            text,
            usage: Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            },
            model: parsed.model.unwrap_or_else(|| body.model.clone()),
        })
    }
}

// ---------------------------------------------------------------------------
// Credentials file format (Codex CLI schema).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    tokens: Option<RawTokens>,
}

#[derive(Debug, Deserialize)]
struct RawTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

fn default_credentials_path() -> Result<PathBuf, ProviderError> {
    if let Ok(p) = std::env::var("CONCLAVE_CODEX_CREDENTIALS") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").map_err(|_| {
        ProviderError::Other("$HOME not set — cannot find Codex credentials".into())
    })?;
    Ok(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn load_credentials(path: &Path) -> Result<OAuthTokens, ProviderError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ProviderError::Other(format!(
                "Codex credentials not found at {} — run `codex login` first",
                path.display()
            ))
        } else {
            ProviderError::Other(format!("read {}: {e}", path.display()))
        }
    })?;
    let parsed: CredentialsFile = serde_json::from_str(&raw)
        .map_err(|e| ProviderError::Other(format!("parse {}: {e}", path.display())))?;
    let raw = parsed.tokens.ok_or_else(|| {
        ProviderError::Other(format!("{} is missing a `tokens` block", path.display()))
    })?;
    Ok(OAuthTokens {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        id_token: raw.id_token,
        account_id: raw.account_id,
    })
}

fn uuid_v4() -> String {
    // Small dependency-free v4 generator using `rand`-style mixing of the
    // current time + a counter. Good enough for a per-request id; we don't
    // need crypto-strong uniqueness here.
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mix = now ^ (counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (mix >> 32) as u32,
        ((mix >> 16) & 0xFFFF) as u16,
        (mix & 0xFFFF) as u16 | 0x4000,       // version 4
        ((counter as u16) & 0x3FFF) | 0x8000, // variant 1
        now & 0xFFFF_FFFF_FFFF
    )
}

// ---------------------------------------------------------------------------
// HTTP wire types — Responses API.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ResponseRequest {
    model: String,
    input: Vec<ResponseInput>,
    store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct ResponseInput {
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    content: Vec<ResponseContent>,
}

#[derive(Serialize, Deserialize)]
struct ResponseContent {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Deserialize)]
struct ResponseEnvelope {
    #[serde(default)]
    output: Vec<ResponseOutputItem>,
    #[serde(default)]
    usage: Option<ResponseUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize)]
struct ResponseOutputItem {
    #[serde(default)]
    content: Option<Vec<ResponseContent>>,
}

#[derive(Deserialize, Default)]
struct ResponseUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sample(path: &Path, account_id: &str) {
        let body = serde_json::json!({
            "tokens": {
                "access_token": "fake-access",
                "refresh_token": "fake-refresh",
                "id_token": "fake-id",
                "account_id": account_id,
            },
        });
        std::fs::write(path, body.to_string()).unwrap();
    }

    #[test]
    fn loads_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("auth.json");
        write_sample(&p, "acct_123");
        let provider = OpenAIOAuthProvider::from_path(&p).unwrap();
        assert_eq!(provider.id(), "openai-oauth");
        assert_eq!(provider.account_id().as_deref(), Some("acct_123"));
    }

    #[test]
    fn missing_credentials_file_errors_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("missing.json");
        let err = OpenAIOAuthProvider::from_path(&p).unwrap_err();
        assert!(err.to_string().contains("codex login"));
    }

    #[test]
    fn uuid_format_is_valid() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
    }
}
