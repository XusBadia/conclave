//! In-app OAuth (PKCE) flows for the subscription providers.
//!
//! Two flavours:
//!
//! - [`AnthropicLoginFlow`]: Anthropic / Claude Max uses a manual
//!   code-paste flow. We open the browser, the user logs in, the
//!   callback page shows them a one-time code, they paste it back into
//!   Conclave and we trade it for tokens.
//! - [`OpenAILoginFlow`]: OpenAI / ChatGPT uses a localhost redirect.
//!   We spin up a one-shot listener on a fixed port, open the browser,
//!   and the callback URL drops the code straight into the listener.
//!
//! After exchange the tokens are persisted to **two** locations:
//!
//! 1. Conclave's own `<config_dir>/oauth/<provider>.json` (always).
//! 2. The Claude Code / Codex CLI credentials file if it already exists
//!    (so the user's other tooling keeps working).
//!
//! Calling code never sees the access token — it just gets a
//! [`LoginOutcome::Ok`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::error::ProviderError;

// ---------------------------------------------------------------------------
// Anthropic / Claude Max — manual code paste.
// ---------------------------------------------------------------------------

/// Client id baked into the official Claude Code CLI. Anthropic publishes
/// no other OAuth client at this time, so we reuse it.
pub const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const ANTHROPIC_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference";

/// State carried between [`AnthropicLoginFlow::start`] and
/// [`AnthropicLoginFlow::complete`].
#[derive(Debug, Clone)]
pub struct AnthropicLoginFlow {
    pkce: PkcePair,
    state: String,
}

impl AnthropicLoginFlow {
    /// Generate PKCE + state and return the URL the user must open.
    pub fn start() -> Result<Started<Self>, ProviderError> {
        let pkce = PkcePair::random();
        let state = random_state();
        let url = build_url(
            ANTHROPIC_AUTHORIZE_URL,
            &[
                ("client_id", ANTHROPIC_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", ANTHROPIC_REDIRECT_URI),
                ("scope", ANTHROPIC_SCOPES),
                ("code_challenge", &pkce.challenge),
                ("code_challenge_method", "S256"),
                ("state", &state),
            ],
        );
        Ok(Started {
            url,
            flow: Self { pkce, state },
        })
    }

    /// Exchange a pasted code (and its state) for tokens. The user copies
    /// the value shown on the Anthropic callback page; the format is
    /// `<code>#<state>` so we also verify CSRF here.
    pub async fn complete(self, raw_code: &str) -> Result<OAuthTokens, ProviderError> {
        let (code, state) = parse_pasted_code(raw_code);
        if let Some(s) = state.as_ref() {
            if s != &self.state {
                return Err(ProviderError::Other(
                    "OAuth state mismatch — restart the login flow".into(),
                ));
            }
        }
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": ANTHROPIC_REDIRECT_URI,
            "client_id": ANTHROPIC_CLIENT_ID,
            "code_verifier": self.pkce.verifier,
            "state": self.state,
        });
        let resp = client
            .post(ANTHROPIC_TOKEN_URL)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "anthropic token exchange failed ({status}): {body_text}"
            )));
        }
        #[derive(Deserialize)]
        struct Resp {
            access_token: String,
            refresh_token: String,
            expires_in: u64,
            #[serde(default)]
            scope: Option<String>,
        }
        let parsed: Resp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("anthropic token parse: {e}")))?;
        Ok(OAuthTokens {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_at_ms: now_ms().saturating_add(parsed.expires_in.saturating_mul(1000)),
            scopes: parsed
                .scope
                .map(|s| s.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            subscription_type: Some("max".into()),
            account_id: None,
        })
    }
}

fn parse_pasted_code(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    if let Some((code, state)) = trimmed.split_once('#') {
        (code.to_owned(), Some(state.to_owned()))
    } else {
        (trimmed.to_owned(), None)
    }
}

// ---------------------------------------------------------------------------
// OpenAI / ChatGPT — localhost redirect.
// ---------------------------------------------------------------------------

/// Public client id used by the official OpenAI Codex CLI.
pub const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Codex CLI uses a fixed port; we re-use it so the redirect URI lines up
/// with what the OpenAI app expects for this client id.
const OPENAI_CALLBACK_PORT: u16 = 1455;
const OPENAI_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
// Scopes (and the four extra query params below) mirror what the official
// Codex CLI sends. The critical one is `codex_cli_simplified_flow=true`:
// without it OpenAI's auth page treats the request as a generic web-OAuth
// hit, and when the user is not already signed into auth.openai.com it
// shows a "Your session has ended" screen whose Log-in link goes to
// `chatgpt.com/auth/login_with?callback_path=/` — discarding the OAuth
// state and dropping the user on chatgpt.com instead of completing the
// redirect. With the simplified-flow flag, OpenAI routes the user into
// `auth.openai.com/log-in` (its first-party login) which preserves the
// authorize context.
const OPENAI_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const OPENAI_ORIGINATOR_PARAM: &str = "codex_cli_rs";

/// State carried between [`OpenAILoginFlow::start`] and
/// [`OpenAILoginFlow::wait_for_callback`].
#[derive(Debug)]
pub struct OpenAILoginFlow {
    pkce: PkcePair,
    state: String,
    listener: TcpListener,
}

impl OpenAILoginFlow {
    /// Bind the localhost listener, generate PKCE/state and return the
    /// authorize URL.
    pub async fn start() -> Result<Started<Self>, ProviderError> {
        let pkce = PkcePair::random();
        let state = random_state();
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], OPENAI_CALLBACK_PORT)))
            .await
            .map_err(|e| {
                ProviderError::Other(format!(
                    "could not bind localhost:{OPENAI_CALLBACK_PORT}: {e} — close any other \
                     instance of Codex/Conclave that may already be listening"
                ))
            })?;
        let url = build_url(
            OPENAI_AUTHORIZE_URL,
            &[
                ("client_id", OPENAI_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", OPENAI_REDIRECT_URI),
                ("scope", OPENAI_SCOPES),
                ("code_challenge", &pkce.challenge),
                ("code_challenge_method", "S256"),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
                ("originator", OPENAI_ORIGINATOR_PARAM),
                ("state", &state),
            ],
        );
        Ok(Started {
            url,
            flow: Self {
                pkce,
                state,
                listener,
            },
        })
    }

    /// Wait for the browser to redirect back to localhost with a code,
    /// exchange it for tokens, and serve a small success page.
    pub async fn wait_for_callback(
        self,
        wait_timeout: Duration,
    ) -> Result<OAuthTokens, ProviderError> {
        let Self {
            pkce,
            state,
            listener,
        } = self;

        let (code, mut stream) = timeout(wait_timeout, async {
            loop {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .map_err(|e| ProviderError::Other(format!("accept: {e}")))?;
                let mut buf = [0u8; 8192];
                let n = stream
                    .read(&mut buf)
                    .await
                    .map_err(|e| ProviderError::Other(format!("read: {e}")))?;
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let Some(path) = req.split_whitespace().nth(1) else {
                    let _ = write_response(&mut stream, "400 Bad Request", "bad request").await;
                    continue;
                };
                let params = parse_callback_query(path);
                if let Some(returned_state) = params.get("state") {
                    if returned_state != &state {
                        let _ = write_response(
                            &mut stream,
                            "400 Bad Request",
                            "state mismatch — restart the login from Conclave",
                        )
                        .await;
                        return Err(ProviderError::Other(
                            "OAuth state mismatch on callback".into(),
                        ));
                    }
                }
                let Some(code) = params.get("code").cloned() else {
                    let _ = write_response(
                        &mut stream,
                        "400 Bad Request",
                        "no code in callback — try again",
                    )
                    .await;
                    continue;
                };
                return Ok::<_, ProviderError>((code, stream));
            }
        })
        .await
        .map_err(|_| ProviderError::Other("OAuth login timed out".into()))??;

        // Serve a friendly success page so the user knows to come back to
        // the app.
        let _ = write_response(
            &mut stream,
            "200 OK",
            "<html><body style=\"font-family: -apple-system, sans-serif; padding: 4rem; \
text-align: center; color: #0b0f14;\"><h1>Conclave — signed in</h1><p>You can close this window.</p></body></html>",
        )
        .await;

        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": OPENAI_REDIRECT_URI,
            "client_id": OPENAI_CLIENT_ID,
            "code_verifier": pkce.verifier,
        });
        let resp = client
            .post(OPENAI_TOKEN_URL)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "openai token exchange failed ({status}): {body_text}"
            )));
        }
        #[derive(Deserialize)]
        struct Resp {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default)]
            id_token: Option<String>,
            #[serde(default)]
            expires_in: Option<u64>,
        }
        let parsed: Resp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("openai token parse: {e}")))?;
        let expires = parsed.expires_in.unwrap_or(60 * 60);
        Ok(OAuthTokens {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token.unwrap_or_default(),
            expires_at_ms: now_ms().saturating_add(expires.saturating_mul(1000)),
            scopes: OPENAI_SCOPES.split_whitespace().map(String::from).collect(),
            subscription_type: None,
            account_id: parsed.id_token,
        })
    }
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
Content-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        len = body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

fn parse_callback_query(path: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let q = match path.split_once('?') {
        Some((_, q)) => q,
        None => return out,
    };
    for kv in q.split('&') {
        let mut it = kv.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        if !k.is_empty() {
            out.insert(url_decode(k), url_decode(v));
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_digit(bytes[i + 1]);
                let lo = hex_digit(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4 | l) as char);
                    i += 3;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

/// Output of `start()` — the URL to open and the live state.
#[derive(Debug)]
pub struct Started<F> {
    pub url: String,
    pub flow: F,
}

/// Tokens returned by any of the OAuth flows. Matches the shape expected
/// by [`crate::AnthropicOAuthProvider`] / [`crate::OpenAIOAuthProvider`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix epoch milliseconds when `access_token` expires.
    pub expires_at_ms: u64,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

/// PKCE verifier + challenge pair.
#[derive(Debug, Clone)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

impl PkcePair {
    /// Generate a 32-byte verifier from the OS RNG; compute its
    /// `S256` challenge.
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
        Self {
            verifier,
            challenge,
        }
    }
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn build_url(base: &str, params: &[(&str, &str)]) -> String {
    let mut url = String::from(base);
    if !params.is_empty() {
        url.push('?');
        for (i, (k, v)) in params.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            url.push_str(&url_encode(k));
            url.push('=');
            url.push_str(&url_encode(v));
        }
    }
    url
}

/// Minimal RFC-3986 percent-encoder — encodes everything that isn't
/// unreserved (`A-Za-z0-9-._~`).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            other => {
                out.push_str(&format!("%{other:02X}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Token storage — Conclave's own location + best-effort CLI compatibility.
// ---------------------------------------------------------------------------

/// Where Conclave stores its own OAuth tokens for `provider_id` under
/// `<config_dir>/oauth/<provider_id>.json`.
pub fn conclave_oauth_path(config_dir: &Path, provider_id: &str) -> PathBuf {
    config_dir.join("oauth").join(format!("{provider_id}.json"))
}

/// Persist tokens both to Conclave's location and (if the file already
/// exists) to the matching Claude Code / Codex CLI credentials file so
/// other tools keep working.
pub fn persist_tokens(
    config_dir: &Path,
    provider_id: &str,
    tokens: &OAuthTokens,
) -> Result<(), ProviderError> {
    let path = conclave_oauth_path(config_dir, provider_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ProviderError::Other(format!("mkdir {}: {e}", parent.display())))?;
    }
    let body = serde_json::to_string_pretty(tokens)
        .map_err(|e| ProviderError::Other(format!("serialise tokens: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| ProviderError::Other(format!("write {}: {e}", path.display())))?;
    // Best-effort: mirror to the CLI tool's credentials file if it exists,
    // matching the schema the official CLI expects.
    if provider_id == "anthropic-oauth" {
        if let Ok(home) = std::env::var("HOME") {
            let cli_path = PathBuf::from(home)
                .join(".claude")
                .join(".credentials.json");
            if cli_path.exists() {
                let payload = serde_json::json!({
                    "claudeAiOauth": {
                        "accessToken": tokens.access_token,
                        "refreshToken": tokens.refresh_token,
                        "expiresAt": tokens.expires_at_ms,
                        "scopes": tokens.scopes,
                        "subscriptionType": tokens.subscription_type,
                    }
                });
                let _ = std::fs::write(
                    &cli_path,
                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                );
            }
        }
    }
    if provider_id == "openai-oauth" {
        if let Ok(home) = std::env::var("HOME") {
            let cli_path = PathBuf::from(home).join(".codex").join("auth.json");
            if cli_path.exists() {
                let payload = serde_json::json!({
                    "tokens": {
                        "access_token": tokens.access_token,
                        "refresh_token": tokens.refresh_token,
                        "id_token": tokens.account_id,
                    },
                });
                let _ = std::fs::write(
                    &cli_path,
                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                );
            }
        }
    }
    Ok(())
}

/// Open `url` in the user's default browser. Returns an error if no
/// browser is available (e.g. on a headless box).
pub fn open_in_browser(url: &str) -> Result<(), ProviderError> {
    open::that(url).map_err(|e| ProviderError::Other(format!("could not open browser: {e}")))
}

/// Re-exported so the `Arc` import is reachable for callers using
/// downstream helpers.
#[allow(dead_code)]
fn _force_arc<T>(_: Arc<T>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_url_safe_b64() {
        let p = PkcePair::random();
        assert!(p
            .verifier
            .chars()
            .all(|c| { matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_') }));
        assert!(p
            .challenge
            .chars()
            .all(|c| { matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_') }));
    }

    #[test]
    fn url_encode_handles_special_chars() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn parse_pasted_code_handles_both_forms() {
        let (code, state) = parse_pasted_code("abc#xyz");
        assert_eq!(code, "abc");
        assert_eq!(state.as_deref(), Some("xyz"));
        let (code, state) = parse_pasted_code(" plain-code ");
        assert_eq!(code, "plain-code");
        assert_eq!(state, None);
    }

    #[test]
    fn callback_query_parses() {
        let q = parse_callback_query("/auth/callback?code=abc&state=xyz&extra=foo");
        assert_eq!(q.get("code").map(String::as_str), Some("abc"));
        assert_eq!(q.get("state").map(String::as_str), Some("xyz"));
        assert_eq!(q.get("extra").map(String::as_str), Some("foo"));
    }

    #[test]
    fn anthropic_start_builds_authorize_url() {
        let started = AnthropicLoginFlow::start().unwrap();
        assert!(started.url.starts_with(ANTHROPIC_AUTHORIZE_URL));
        assert!(started.url.contains("client_id"));
        assert!(started.url.contains("code_challenge_method=S256"));
        assert!(started.url.contains("response_type=code"));
    }
}
