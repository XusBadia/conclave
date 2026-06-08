//! Anthropic OAuth — uses the credentials dropped on disk by the official
//! Claude Code CLI (`claude login`). Lets a user point Conclave at their
//! Claude Max subscription instead of an API key.
//!
//! **Experimental.** Anthropic does not publicly document the OAuth flow
//! and the file layout / endpoints can change between releases.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const REFRESH_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_MODEL: &str = "claude-sonnet-4-6-20250929";
const ANTHROPIC_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-cli/2.0 (Conclave)";

/// OAuth provider backed by the Claude Code CLI's credentials file.
pub struct AnthropicOAuthProvider {
    credentials_path: PathBuf,
    base_url: String,
    default_model: String,
    client: reqwest::Client,
    cached: Mutex<Option<OAuthCredentials>>,
}

impl std::fmt::Debug for AnthropicOAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicOAuthProvider")
            .field("credentials_path", &self.credentials_path)
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish_non_exhaustive()
    }
}

impl AnthropicOAuthProvider {
    /// Build a provider that reads `~/.claude/.credentials.json` (or whatever
    /// `CONCLAVE_CLAUDE_CREDENTIALS` env var points to).
    pub fn from_default_location() -> Result<Self, ProviderError> {
        let path = default_credentials_path()?;
        Self::from_path(path)
    }

    /// Build with an explicit credentials file path.
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let path = path.into();
        let credentials = load_credentials(&path)?;
        Ok(Self {
            credentials_path: path,
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: crate::cli_local::http_client(),
            cached: Mutex::new(Some(credentials)),
        })
    }

    /// Build from Conclave's own OAuth store (preferred) with a fallback
    /// to the Claude Code CLI credentials file. This is the path used by
    /// the in-app login flow.
    pub fn from_conclave_or_cli(config_dir: &Path) -> Result<Self, ProviderError> {
        let conclave_path = config_dir.join("oauth").join("anthropic-oauth.json");
        if conclave_path.exists() {
            return Self::from_conclave_tokens(conclave_path);
        }
        Self::from_default_location()
    }

    /// Build from Conclave's `<config_dir>/oauth/anthropic-oauth.json`.
    pub fn from_conclave_tokens(path: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let path = path.into();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| ProviderError::Other(format!("read {}: {e}", path.display())))?;
        #[derive(serde::Deserialize)]
        struct Stored {
            access_token: String,
            refresh_token: String,
            expires_at_ms: u64,
            #[serde(default)]
            scopes: Vec<String>,
            #[serde(default)]
            subscription_type: Option<String>,
        }
        let parsed: Stored = serde_json::from_str(&raw)
            .map_err(|e| ProviderError::Other(format!("parse {}: {e}", path.display())))?;
        let credentials = OAuthCredentials {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_at: parsed.expires_at_ms,
            scopes: parsed.scopes,
            subscription_type: parsed.subscription_type,
        };
        Ok(Self {
            credentials_path: path,
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: crate::cli_local::http_client(),
            cached: Mutex::new(Some(credentials)),
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

    /// Subscription type reported by the cached credentials, if known.
    pub fn subscription_type(&self) -> Option<String> {
        self.cached
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|c| c.subscription_type.clone()))
    }

    /// Ensure we have a non-expired access token, refreshing if needed.
    async fn ensure_token(&self) -> Result<String, ProviderError> {
        let snapshot = {
            let guard = self
                .cached
                .lock()
                .map_err(|_| ProviderError::Other("oauth cache poisoned".into()))?;
            guard.clone()
        };
        let Some(credentials) = snapshot else {
            let fresh = load_credentials(&self.credentials_path)?;
            let token = fresh.access_token.clone();
            if let Ok(mut g) = self.cached.lock() {
                *g = Some(fresh);
            }
            return Ok(token);
        };

        // Refresh with 60s of safety margin.
        let now_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);
        if credentials.expires_at <= now_ms.saturating_add(60_000) {
            tracing::info!("anthropic oauth token expired or near-expiry — refreshing");
            let refreshed = self.refresh(&credentials.refresh_token).await?;
            let token = refreshed.access_token.clone();
            persist_credentials(&self.credentials_path, &refreshed)?;
            if let Ok(mut g) = self.cached.lock() {
                *g = Some(refreshed);
            }
            return Ok(token);
        }

        Ok(credentials.access_token)
    }

    /// Lightweight reachability + auth check. Sends a 1-token request
    /// to `/v1/messages` and inspects the HTTP status — returns
    /// `Ok(())` when the bearer is accepted, `Err(Auth)` on 401/403,
    /// `Err(Network)` on transport failures. We reuse `ensure_token`
    /// so an expired access token gets refreshed before the probe;
    /// only a *revoked* refresh token (or a missing credential) ends
    /// up here as `Auth`.
    pub async fn probe(&self) -> Result<(), ProviderError> {
        let access_token = self.ensure_token().await?;
        let body = OauthRequest {
            model: self.default_model.clone(),
            messages: vec![OauthMessage {
                role: "user",
                content: OauthContentField::Text("ping".to_owned()),
            }],
            system: None,
            max_tokens: 1,
            temperature: None,
        };
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {access_token}"))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", ANTHROPIC_BETA)
            .header("user-agent", USER_AGENT)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderError::Auth);
        }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("{status}: {body_text}")));
        }
        drop(resp);
        Ok(())
    }

    async fn refresh(&self, refresh_token: &str) -> Result<OAuthCredentials, ProviderError> {
        #[derive(Serialize)]
        struct Body<'a> {
            grant_type: &'a str,
            refresh_token: &'a str,
            client_id: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            access_token: String,
            refresh_token: String,
            expires_in: u64,
            #[serde(default)]
            scope: Option<String>,
        }
        let resp = self
            .client
            .post(REFRESH_URL)
            .json(&Body {
                grant_type: "refresh_token",
                refresh_token,
                client_id: CLIENT_ID,
            })
            .send()
            .await
            .map_err(|e| ProviderError::Network(format!("anthropic refresh: {e}")))?;
        if !resp.status().is_success() {
            let _status = resp.status();
            let _body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Auth);
        }
        let parsed: Resp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("anthropic refresh parse: {e}")))?;
        let now_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);
        let scopes = parsed
            .scope
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        Ok(OAuthCredentials {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_at: now_ms.saturating_add(parsed.expires_in.saturating_mul(1000)),
            scopes,
            subscription_type: None,
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicOAuthProvider {
    fn id(&self) -> &'static str {
        "anthropic-oauth"
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
        let mut system_parts = Vec::new();
        let mut convo: Vec<OauthMessage> = Vec::new();
        for m in req.messages {
            match m.role {
                MessageRole::System => system_parts.push(m.content),
                MessageRole::User => convo.push(OauthMessage {
                    role: "user",
                    content: OauthContentField::Text(m.content),
                }),
                MessageRole::Assistant => convo.push(OauthMessage {
                    role: "assistant",
                    content: OauthContentField::Text(m.content),
                }),
            }
        }
        if convo.is_empty() {
            return Err(ProviderError::BadRequest(
                "anthropic-oauth requires at least one user/assistant message".into(),
            ));
        }

        // Attach images to the last user message when present.
        if !req.images.is_empty() {
            let last_user_idx = convo
                .iter()
                .rposition(|m| m.role == "user")
                .unwrap_or(convo.len());
            if last_user_idx == convo.len() {
                convo.push(OauthMessage {
                    role: "user",
                    content: OauthContentField::Blocks(Vec::new()),
                });
            }
            convo[last_user_idx].promote_to_blocks();
            for img in &req.images {
                convo[last_user_idx].push_block(OauthBlock::Image {
                    source: OauthImageSource {
                        kind: "base64",
                        media_type: img.media_type.clone(),
                        data: img.base64_data.clone(),
                    },
                });
            }
        }

        let model = if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        };
        let body = OauthRequest {
            model,
            messages: convo,
            system: if system_parts.is_empty() {
                None
            } else {
                Some(system_parts.join("\n\n"))
            },
            max_tokens: req.max_output_tokens.unwrap_or(4096),
            temperature: req.temperature,
        };

        let access_token = self.ensure_token().await?;
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("authorization", format!("Bearer {access_token}"))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", ANTHROPIC_BETA)
            .header("user-agent", USER_AGENT)
            .header("content-type", "application/json")
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

        let parsed: OauthResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("anthropic-oauth parse: {e}")))?;

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

// ---------------------------------------------------------------------------
// Credentials file format (Claude Code v2 schema).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OAuthCredentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix epoch milliseconds when the access token expires.
    pub expires_at: u64,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<RawCredentials>,
}

#[derive(Debug, Deserialize)]
struct RawCredentials {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "expiresAt")]
    expires_at: u64,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(rename = "subscriptionType", default)]
    subscription_type: Option<String>,
}

fn default_credentials_path() -> Result<PathBuf, ProviderError> {
    if let Ok(p) = std::env::var("CONCLAVE_CLAUDE_CREDENTIALS") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").map_err(|_| {
        ProviderError::Other("$HOME not set — cannot find Claude credentials".into())
    })?;
    Ok(PathBuf::from(home)
        .join(".claude")
        .join(".credentials.json"))
}

fn load_credentials(path: &Path) -> Result<OAuthCredentials, ProviderError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ProviderError::Other(format!(
                "Claude credentials not found at {} — run `claude login` first",
                path.display()
            ))
        } else {
            ProviderError::Other(format!("read {}: {e}", path.display()))
        }
    })?;
    let parsed: CredentialsFile = serde_json::from_str(&raw)
        .map_err(|e| ProviderError::Other(format!("parse {}: {e}", path.display())))?;
    let raw = parsed.claude_ai_oauth.ok_or_else(|| {
        ProviderError::Other(format!(
            "{} is missing the `claudeAiOauth` block",
            path.display()
        ))
    })?;
    Ok(OAuthCredentials {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        expires_at: raw.expires_at,
        scopes: raw.scopes,
        subscription_type: raw.subscription_type,
    })
}

fn persist_credentials(path: &Path, creds: &OAuthCredentials) -> Result<(), ProviderError> {
    #[derive(Serialize)]
    struct Out<'a> {
        #[serde(rename = "claudeAiOauth")]
        claude_ai_oauth: RawOut<'a>,
    }
    #[derive(Serialize)]
    struct RawOut<'a> {
        #[serde(rename = "accessToken")]
        access_token: &'a str,
        #[serde(rename = "refreshToken")]
        refresh_token: &'a str,
        #[serde(rename = "expiresAt")]
        expires_at: u64,
        scopes: &'a [String],
        #[serde(rename = "subscriptionType", skip_serializing_if = "Option::is_none")]
        subscription_type: Option<&'a str>,
    }
    let out = Out {
        claude_ai_oauth: RawOut {
            access_token: &creds.access_token,
            refresh_token: &creds.refresh_token,
            expires_at: creds.expires_at,
            scopes: &creds.scopes,
            subscription_type: creds.subscription_type.as_deref(),
        },
    };
    let body = serde_json::to_string_pretty(&out)
        .map_err(|e| ProviderError::Other(format!("serialise creds: {e}")))?;
    std::fs::write(path, body)
        .map_err(|e| ProviderError::Other(format!("write {}: {e}", path.display())))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OauthRequest {
    model: String,
    messages: Vec<OauthMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct OauthMessage {
    role: &'static str,
    content: OauthContentField,
}

#[derive(Serialize)]
#[serde(untagged)]
enum OauthContentField {
    Text(String),
    Blocks(Vec<OauthBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OauthBlock {
    Text { text: String },
    Image { source: OauthImageSource },
}

#[derive(Serialize)]
struct OauthImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: String,
    data: String,
}

impl OauthMessage {
    fn promote_to_blocks(&mut self) {
        if let OauthContentField::Text(t) = &self.content {
            let initial = if t.is_empty() {
                Vec::new()
            } else {
                vec![OauthBlock::Text { text: t.clone() }]
            };
            self.content = OauthContentField::Blocks(initial);
        }
    }
    fn push_block(&mut self, block: OauthBlock) {
        if let OauthContentField::Blocks(blocks) = &mut self.content {
            blocks.push(block);
        }
    }
}

#[derive(Deserialize)]
struct OauthResponse {
    content: Vec<OauthContent>,
    model: String,
    usage: OauthUsage,
}

#[derive(Deserialize)]
struct OauthContent {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct OauthUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sample(path: &Path, expires_at: u64) {
        let body = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-test",
                "refreshToken": "sk-ant-ort01-test",
                "expiresAt": expires_at,
                "scopes": ["user:inference"],
                "subscriptionType": "max"
            }
        });
        std::fs::write(path, body.to_string()).unwrap();
    }

    #[test]
    fn loads_credentials_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(".credentials.json");
        write_sample(&p, u64::MAX);
        let provider = AnthropicOAuthProvider::from_path(&p).unwrap();
        assert_eq!(provider.id(), "anthropic-oauth");
        assert_eq!(provider.subscription_type().as_deref(), Some("max"));
    }

    #[test]
    fn missing_credentials_file_errors_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("absent.json");
        let err = AnthropicOAuthProvider::from_path(&p).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("claude login"), "{msg}");
    }
}
