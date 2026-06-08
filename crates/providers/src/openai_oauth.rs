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
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace, warn};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
    WebCitation,
};
use crate::LlmProvider;

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
// `gpt-5.5` is the recommended Codex-on-ChatGPT model as of May 2026
// (per developers.openai.com/codex/models). `gpt-5-codex` and plain
// `gpt-5` would either reject the ChatGPT-account token or fall back
// to an older variant. Keep this in sync with the Codex CLI default.
const DEFAULT_MODEL: &str = "gpt-5.5";
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
            client: crate::cli_local::http_client(),
            cached: Mutex::new(Some(tokens)),
        })
    }

    /// Build from Conclave's own OAuth store (preferred) with a fallback
    /// to the Codex CLI credentials file.
    pub fn from_conclave_or_cli(config_dir: &Path) -> Result<Self, ProviderError> {
        let conclave_path = config_dir.join("oauth").join("openai-oauth.json");
        if conclave_path.exists() {
            return Self::from_conclave_tokens(conclave_path);
        }
        Self::from_default_location()
    }

    /// Build from Conclave's `<config_dir>/oauth/openai-oauth.json`.
    pub fn from_conclave_tokens(path: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let path = path.into();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| ProviderError::Other(format!("read {}: {e}", path.display())))?;
        #[derive(serde::Deserialize)]
        struct Stored {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default)]
            account_id: Option<String>,
        }
        let parsed: Stored = serde_json::from_str(&raw)
            .map_err(|e| ProviderError::Other(format!("parse {}: {e}", path.display())))?;
        let tokens = OAuthTokens {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            id_token: None,
            account_id: parsed.account_id,
        };
        Ok(Self {
            credentials_path: path,
            base_url: DEFAULT_BASE_URL.to_owned(),
            default_model: DEFAULT_MODEL.to_owned(),
            client: crate::cli_local::http_client(),
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

    /// A short, human-readable label for the signed-in account, suitable
    /// for the Settings card. When tokens come from our own OAuth flow
    /// the `account_id` field actually stores the raw OpenID `id_token`
    /// (a JWT); we decode its payload and surface the user's email. When
    /// tokens come from the Codex CLI credentials file it stores an
    /// opaque `acct_…` id; we return that unchanged. Falls back to
    /// `None` when neither shape applies.
    pub fn account_label(&self) -> Option<String> {
        let raw = self
            .cached
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|t| t.account_id.clone()))?;
        if let Some(email) = jwt_claim_email(&raw) {
            return Some(email);
        }
        // Not a JWT — assume CLI account id and pass through unchanged.
        // Skip anything that looks like a JWT we failed to parse so the
        // UI never displays a giant base64 blob.
        if raw.matches('.').count() == 2 {
            return None;
        }
        Some(raw)
    }

    /// Back-compat alias. The previous name suggested it always returned
    /// an account id, but for tokens produced by the in-app OAuth flow it
    /// returned the raw `id_token`. Prefer [`Self::account_label`].
    #[doc(hidden)]
    pub fn account_id(&self) -> Option<String> {
        self.account_label()
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

    /// Lightweight reachability + auth check. Sends a 1-token request to
    /// the same `/codex/responses` endpoint `complete()` uses, but bails
    /// out as soon as the HTTP status arrives — we don't care about the
    /// SSE body. Returns `Ok(())` when the bearer token is accepted,
    /// `Err(ProviderError::Auth)` on 401/403, `Err(ProviderError::Network)`
    /// on transport failures.
    ///
    /// The cost is one input token of the user's ChatGPT quota; the
    /// commands-layer cache keeps repeated UI refreshes from spamming
    /// this. We hit the *real* endpoint (not a hypothetical health
    /// route) so a green probe directly implies a green committee run.
    pub async fn probe(&self) -> Result<(), ProviderError> {
        let token = self.current_token()?;
        let body = ResponseRequest {
            model: self.default_model.clone(),
            instructions: "ping".to_owned(),
            input: vec![ResponseInput {
                kind: "message",
                role: "user",
                content: vec![ResponseContent {
                    kind: "input_text".to_owned(),
                    text: "ping".to_owned(),
                    annotations: Vec::new(),
                }],
            }],
            store: false,
            stream: true,
            tools: Vec::new(),
        };
        let url = format!("{}/codex/responses", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("OpenAI-Beta", "responses=v1")
            .header("originator", ORIGINATOR)
            .header("session_id", uuid_v4())
            .header("user-agent", USER_AGENT)
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
        // Drop the response without consuming the SSE body — the
        // connection closes cleanly when the handle goes out of scope.
        drop(resp);
        Ok(())
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
            scope: ProviderScope::General,
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

        // The Codex Responses API requires a top-level `instructions` field
        // separate from the `input` messages. Collect any System messages
        // here; the remaining messages become the input array. If the caller
        // didn't supply a system message, fall back to a minimal default so
        // the API doesn't reject the request with "Instructions are required".
        let (system_parts, other_messages): (Vec<_>, Vec<_>) = req
            .messages
            .into_iter()
            .partition(|m| matches!(m.role, MessageRole::System));
        let instructions = if system_parts.is_empty() {
            "You are a helpful assistant. Follow the user's instructions.".to_owned()
        } else {
            system_parts
                .into_iter()
                .map(|m| m.content)
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        // Build the input array the Responses API expects.
        let input: Vec<ResponseInput> = other_messages
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
                    annotations: Vec::new(),
                }],
            })
            .collect();
        if input.is_empty() {
            return Err(ProviderError::BadRequest(
                "openai-oauth requires at least one non-system message".into(),
            ));
        }

        // NOTE: We do NOT advertise `web_search_preview` here. The Codex
        // endpoint at chatgpt.com/backend-api/codex/responses only supports
        // Codex's own coding tools (`shell`, `apply_patch`, …) and rejects
        // `web_search_preview` with `400 Unsupported tool type`. The Q&A
        // pipeline runs its own DuckDuckGo search and injects results into
        // the prompt before calling us — `req.allow_web_search` is therefore
        // ignored by this provider.
        let _ = req.allow_web_search;
        let tools: Vec<ResponseTool> = Vec::new();

        let body = ResponseRequest {
            model,
            instructions,
            input,
            store: false,
            // The Codex Responses API enforces streaming for OAuth callers
            // — non-streaming requests return `400 "Stream must be set to true"`.
            // We still surface a single final CompletionResponse to the caller;
            // streaming is purely a wire-level concern handled below.
            stream: true,
            tools,
            // NOTE: Codex's `/codex/responses` endpoint rejects both
            // `max_output_tokens` and `temperature` with `400 Unsupported
            // parameter`, even though the upstream OpenAI Responses API
            // accepts them. We drop them for Codex callers; the API picks
            // sensible defaults.
        };

        let session_id = uuid_v4();
        let url = format!("{}/codex/responses", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
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

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let content_length = resp.content_length();
        debug!(
            target: "conclave_providers::openai_oauth",
            status = %resp.status(),
            content_type = %content_type,
            content_length = ?content_length,
            "openai-oauth response headers received"
        );

        // Consume the SSE body chunk-by-chunk via `bytes_stream` instead of
        // `resp.text().await`. The Codex Responses endpoint can drop the
        // connection mid-stream on the finalize phase (long output) — when
        // that happens the raw `text()` call returns reqwest's opaque
        // "error decoding response body" with no way to tell whether we
        // already had enough events to answer. Streaming lets us hold on
        // to whatever arrived, log byte/event counts, and still recover
        // if the terminal `response.completed` event made it through.
        let mut byte_buf: Vec<u8> = Vec::with_capacity(8 * 1024);
        let mut stream = resp.bytes_stream();
        let mut interrupt_err: Option<reqwest::Error> = None;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    byte_buf.extend_from_slice(&bytes);
                    trace!(
                        target: "conclave_providers::openai_oauth",
                        chunk_bytes = bytes.len(),
                        total_bytes = byte_buf.len(),
                        "openai-oauth chunk"
                    );
                }
                Err(e) => {
                    interrupt_err = Some(e);
                    break;
                }
            }
        }

        // SSE is always UTF-8 per the spec; lossy conversion only kicks in
        // on truncation at a multi-byte boundary, which we'd flag as an
        // interrupted stream anyway.
        let body_text = String::from_utf8_lossy(&byte_buf).into_owned();
        let parse_result = aggregate_sse(&body_text);

        if let Some(err) = interrupt_err {
            let events_seen = parse_result.as_ref().map(|a| a.events_seen).unwrap_or(0);
            let saw_terminal = parse_result.as_ref().is_ok_and(|a| a.saw_terminal_event);
            if saw_terminal {
                warn!(
                    target: "conclave_providers::openai_oauth",
                    bytes = byte_buf.len(),
                    events = events_seen,
                    err = %err,
                    "openai-oauth stream interrupted after terminal event; returning aggregated response"
                );
            } else {
                warn!(
                    target: "conclave_providers::openai_oauth",
                    bytes = byte_buf.len(),
                    events = events_seen,
                    err = %err,
                    "openai-oauth stream interrupted before terminal event"
                );
                return Err(ProviderError::Network(format!(
                    "openai-oauth stream interrupted bytes={} events={}: {}",
                    byte_buf.len(),
                    events_seen,
                    err
                )));
            }
        }

        let aggregated =
            parse_result.map_err(|e| ProviderError::Other(format!("openai-oauth parse: {e}")))?;
        debug!(
            target: "conclave_providers::openai_oauth",
            bytes = byte_buf.len(),
            events = aggregated.events_seen,
            input_tokens = aggregated.input_tokens,
            output_tokens = aggregated.output_tokens,
            "openai-oauth response aggregated"
        );

        Ok(CompletionResponse {
            text: aggregated.text,
            usage: Usage {
                input_tokens: aggregated.input_tokens,
                output_tokens: aggregated.output_tokens,
            },
            model: aggregated.model.unwrap_or_else(|| body.model.clone()),
            web_citations: aggregated.web_citations,
        })
    }
}

/// Walks the SSE event stream from the Codex Responses API and rolls every
/// `response.output_text.delta` chunk into a single string. The
/// `response.completed` event carries the authoritative usage counters
/// and (optionally) the final assembled output, which we prefer over the
/// delta concatenation if present.
struct AggregatedResponse {
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    model: Option<String>,
    web_citations: Vec<WebCitation>,
    /// Number of SSE events whose JSON payload was successfully parsed.
    /// Used by the caller for stream-interruption diagnostics.
    events_seen: u32,
    /// `true` if we processed a `response.completed` or `response.done`
    /// event. When the stream is cut off after this, we still have a
    /// usable response and can return success.
    saw_terminal_event: bool,
}

fn aggregate_sse(body: &str) -> Result<AggregatedResponse, String> {
    let mut deltas = String::new();
    let mut final_text: Option<String> = None;
    let mut usage = ResponseUsage::default();
    let mut model: Option<String> = None;
    let mut web_citations: Vec<WebCitation> = Vec::new();
    let mut saw_any_event = false;
    let mut events_seen: u32 = 0;
    let mut saw_terminal_event = false;

    // SSE events are separated by blank lines. Within an event, "data:"
    // lines carry the JSON payload — we ignore "event:" hints because the
    // `type` field inside the payload is authoritative.
    for block in body.split("\n\n") {
        let mut payload = String::new();
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !payload.is_empty() {
                    payload.push('\n');
                }
                payload.push_str(rest.trim_start());
            }
        }
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        saw_any_event = true;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            // Truncated JSON at the tail of an interrupted stream lands
            // here — `saw_any_event` stays true but we don't bump the
            // parsed counter.
            continue;
        };
        events_seen = events_seen.saturating_add(1);
        let ty = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "response.output_text.delta" => {
                if let Some(d) = value.get("delta").and_then(|v| v.as_str()) {
                    deltas.push_str(d);
                }
            }
            "response.output_text.annotation.added" => {
                if let Some(ann) = value.get("annotation") {
                    if let Ok(parsed) = serde_json::from_value::<ResponseAnnotation>(ann.clone()) {
                        push_web_citation(&mut web_citations, &parsed);
                    }
                }
            }
            "response.completed" | "response.done" => {
                saw_terminal_event = true;
                if let Some(response) = value.get("response") {
                    if let Ok(env) = serde_json::from_value::<ResponseEnvelope>(response.clone()) {
                        let text = env
                            .output
                            .iter()
                            .flat_map(|o| o.content.as_deref().unwrap_or(&[]))
                            .filter(|c| c.kind == "output_text" || c.kind == "text")
                            .map(|c| c.text.clone())
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            final_text = Some(text);
                        }
                        for content in env
                            .output
                            .iter()
                            .flat_map(|o| o.content.as_deref().unwrap_or(&[]))
                        {
                            for ann in &content.annotations {
                                push_web_citation(&mut web_citations, ann);
                            }
                        }
                        if let Some(u) = env.usage {
                            usage = u;
                        }
                        if env.model.is_some() {
                            model = env.model;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if !saw_any_event {
        return Err("no SSE events received".into());
    }

    let text = final_text.unwrap_or(deltas);
    Ok(AggregatedResponse {
        text,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        model,
        web_citations,
        events_seen,
        saw_terminal_event,
    })
}

/// Append a citation if it points to a URL and hasn't been recorded yet.
/// Codex sometimes emits the same URL repeatedly (once per sentence the
/// model attributes to it); dedupe by URL so the UI doesn't list seven
/// copies of the same page.
fn push_web_citation(out: &mut Vec<WebCitation>, ann: &ResponseAnnotation) {
    if ann.kind != "url_citation" {
        return;
    }
    let Some(url) = ann.url.as_deref().filter(|u| !u.is_empty()) else {
        return;
    };
    if out.iter().any(|c| c.url == url) {
        return;
    }
    out.push(WebCitation {
        url: url.to_owned(),
        title: ann.title.clone().unwrap_or_default(),
        snippet: String::new(),
    });
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

/// Extract the `email` claim from a JWT-shaped string, or `None` if the
/// input isn't a JWT, the payload doesn't decode, or no email claim is
/// present. Verifies nothing — we only use this to label the connected
/// account in the UI.
fn jwt_claim_email(token: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    // Standard `email` claim first; fall back to nested OpenAI profile
    // claims if upstream tweaks the shape.
    claims
        .get("email")
        .and_then(|v| v.as_str())
        .or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(|p| p.get("email"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| claims.get("name").and_then(|v| v.as_str()))
        .map(str::to_owned)
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
    instructions: String,
    input: Vec<ResponseInput>,
    store: bool,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ResponseTool>,
}

#[derive(Serialize)]
struct ResponseTool {
    #[serde(rename = "type")]
    kind: &'static str,
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
    #[serde(default)]
    text: String,
    /// Inline citations attached to this text segment when the model
    /// invoked a tool (e.g. `web_search_preview`). Only meaningful on
    /// deserialization — never serialized back.
    #[serde(default, skip_serializing)]
    annotations: Vec<ResponseAnnotation>,
}

#[derive(Deserialize, Default, Clone)]
struct ResponseAnnotation {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
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

    #[test]
    fn account_label_extracts_email_from_jwt() {
        // Hand-crafted JWT: header.payload.sig where payload claims
        // `{"email":"dr@example.com"}`. Signature is irrelevant — we
        // don't verify, only decode.
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"dr@example.com","sub":"u_1"}"#);
        let jwt = format!("{header}.{payload}.sig");
        assert_eq!(jwt_claim_email(&jwt).as_deref(), Some("dr@example.com"));
    }

    #[test]
    fn account_label_falls_back_to_name_when_no_email() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(br#"{"name":"Dr Who","sub":"u_1"}"#);
        let jwt = format!("{header}.{payload}.sig");
        assert_eq!(jwt_claim_email(&jwt).as_deref(), Some("Dr Who"));
    }

    #[test]
    fn account_label_returns_none_for_unparseable_jwt() {
        // Three dot-separated segments but the middle isn't valid base64.
        let result = jwt_claim_email("aaa.???.zzz");
        assert!(result.is_none());
    }

    #[test]
    fn aggregate_sse_accumulates_deltas_and_completes_with_usage() {
        // Simulates a minimal Codex SSE stream: two text deltas followed by
        // a `response.completed` event with usage. The final text comes from
        // the completed event when present.
        let sse = "\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello \"}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"content\":[{\"type\":\"output_text\",\"text\":\"Hello world\"}]}],\"usage\":{\"input_tokens\":42,\"output_tokens\":7},\"model\":\"gpt-5.5\"}}\n\
\n\
data: [DONE]\n\
\n";
        let agg = aggregate_sse(sse).unwrap();
        assert_eq!(agg.text, "Hello world");
        assert_eq!(agg.input_tokens, 42);
        assert_eq!(agg.output_tokens, 7);
        assert_eq!(agg.model.as_deref(), Some("gpt-5.5"));
        assert!(agg.saw_terminal_event);
        assert_eq!(agg.events_seen, 3);
    }

    #[test]
    fn aggregate_sse_falls_back_to_deltas_when_no_completed_event() {
        // Some servers may close before emitting `response.completed`; the
        // accumulated delta string should still be returned.
        let sse = "\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\
\n";
        let agg = aggregate_sse(sse).unwrap();
        assert_eq!(agg.text, "partial");
        assert_eq!(agg.input_tokens, 0);
        assert!(agg.model.is_none());
        assert!(!agg.saw_terminal_event);
        assert_eq!(agg.events_seen, 1);
    }

    #[test]
    fn aggregate_sse_handles_truncated_event_at_tail() {
        // Simulates a stream cut off mid-event: two complete deltas followed
        // by a third event whose JSON payload is truncated. The caller
        // (complete()) uses `events_seen` + `saw_terminal_event` to decide
        // whether the interruption is recoverable.
        let sse = "\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\
\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\
\n\
data: {\"type\":\"response.output_text.delt";
        let agg = aggregate_sse(sse).unwrap();
        assert_eq!(agg.text, "Hello");
        assert!(!agg.saw_terminal_event);
        // Two complete events parsed; the truncated tail is skipped.
        assert_eq!(agg.events_seen, 2);
    }

    #[test]
    fn aggregate_sse_errors_when_stream_is_empty() {
        let agg = aggregate_sse("");
        assert!(agg.is_err());
    }

    #[test]
    fn account_label_returns_raw_for_non_jwt_account_id() {
        // CLI-style account ids look like `acct_xxx` and have no dots —
        // they should pass through unchanged.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("auth.json");
        write_sample(&p, "acct_xyz");
        let provider = OpenAIOAuthProvider::from_path(&p).unwrap();
        assert_eq!(provider.account_label().as_deref(), Some("acct_xyz"));
    }

    #[test]
    fn account_label_hides_unparseable_jwt_instead_of_dumping_it() {
        // What `~/.codex/auth.json` looks like for users whose account_id
        // field was populated with an id_token we can't decode — should
        // hide rather than dump the entire base64 blob in the UI.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("auth.json");
        write_sample(&p, "eyJhbGc.???.zzz");
        let provider = OpenAIOAuthProvider::from_path(&p).unwrap();
        assert!(provider.account_label().is_none());
    }
}
