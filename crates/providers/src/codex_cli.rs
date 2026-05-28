//! Codex CLI provider — proxies completions through the user's
//! locally installed `codex` binary in non-interactive `exec` mode.
//!
//! ## Why this exists
//!
//! Same rationale as [`crate::claude_cli`]: OpenAI does not publish a
//! third-party OAuth client for ChatGPT Plus / Pro subscriptions, and
//! reusing the Codex CLI's own client id from outside the CLI is at
//! best a grey area. `codex exec` is the documented programmatic
//! entry point ("exits non-zero if submission fails so you can wire
//! it into scripts or CI"). Shelling out to the user's locally
//! installed and locally authenticated `codex` is therefore the only
//! vendor-sanctioned way for Conclave to use a Codex / ChatGPT
//! subscription.
//!
//! ## How it works
//!
//! - At construction time we resolve `codex` in `$PATH` and cache the
//!   result. Missing → the provider self-reports as unavailable.
//! - `complete()` spawns:
//!   ```text
//!   codex exec
//!     --sandbox read-only
//!     --skip-git-repo-check
//!     --ephemeral
//!     --ignore-user-config
//!     --ignore-rules
//!     --color never
//!     --output-last-message <tempfile>
//!     "<flattened conversation>"
//!   ```
//! - Output capture: `--output-last-message` writes the agent's final
//!   message to a file we create in our scratch `TempDir`. We read
//!   that file rather than parsing stdout (which is interleaved with
//!   the CLI's session header, token-usage footer, and progress
//!   lines).
//! - Sandbox: `--sandbox read-only` is the safest mode in `exec`.
//!   Passed explicitly so a user's `config.toml` cannot widen
//!   permissions silently.
//! - Isolation flags: `--skip-git-repo-check` (the TempDir is not a
//!   git repo), `--ephemeral` (no session file on disk),
//!   `--ignore-user-config` (skip `~/.codex/config.toml` —
//!   authentication still uses `CODEX_HOME`), `--ignore-rules` (no
//!   execpolicy `.rules`). Together they keep the agent's behaviour
//!   reproducible regardless of the host user's Codex setup.
//! - CWD is a freshly created `TempDir` so a stray `AGENTS.md` /
//!   `.codex/` folder in the host project cannot leak into the
//!   prompt.
//!
//! ## Limitations
//!
//! - Token counts: the CLI does not surface them on stdout, so
//!   [`Usage`] is reported as zeros.
//! - System prompt: `codex exec` has no flag for a custom system
//!   prompt. We flatten the request's system content into the user
//!   prompt with an `Instructions:` / `Request:` separator. The CLI
//!   itself adds OpenAI's standard Codex prompting on top of that.
//! - Vision: not supported in `exec` mode per the docs.
//! - Web citations: not surfaced.
//! - Model selection: the `--model` flag's behaviour in `exec` is not
//!   documented; we let Codex pick from `~/.codex/config.toml`. The
//!   nominal default reported to the UI is `gpt-5`.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

/// Nominal model id reported to the UI.
///
/// The actual model used at runtime is whatever Codex's built-in
/// default selects — we pass `--ignore-user-config` to keep the choice
/// deterministic, and at the time of writing Codex 0.128 ships
/// `gpt-5.5` as that default.
pub const DEFAULT_MODEL: &str = "gpt-5.5";

/// Hard ceiling on a single completion. `codex exec` does not enforce
/// a built-in timeout; this prevents wedged batches.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(180);

/// Stable id used by the registry and `ProviderInfo.id`.
pub const PROVIDER_ID: &str = "codex-cli";

/// Provider that proxies a completion request to the local `codex`
/// binary via `codex exec`.
#[derive(Clone)]
pub struct CodexCliProvider {
    /// Pre-resolved path to the `codex` binary, when present on
    /// `$PATH`. `None` means not found at construction time.
    binary: Option<PathBuf>,
}

impl std::fmt::Debug for CodexCliProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCliProvider")
            .field("binary", &self.binary)
            .finish()
    }
}

impl Default for CodexCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCliProvider {
    /// Construct with cached resolution of `codex` in `$PATH`.
    pub fn new() -> Self {
        Self {
            binary: detect_cached(),
        }
    }

    /// Process-wide cached resolution of the `codex` binary path.
    pub fn binary_path() -> Option<PathBuf> {
        detect_cached()
    }

    /// `true` when `codex` is present on `$PATH`.
    pub fn is_installed() -> bool {
        detect_cached().is_some()
    }

    /// Invalidate the memoised binary path so the next call re-walks
    /// `$PATH`. Mirror of [`crate::ClaudeCliProvider::refresh_binary_cache`].
    pub fn refresh_binary_cache() {
        if let Ok(mut guard) = CACHED.write() {
            *guard = BinaryCache::Unprobed;
        }
    }

    /// Probe `codex login status` — the vendor-documented way to
    /// check whether the user has an active session ("Show login
    /// status"; exits 0 when logged in). Falls back to inspecting
    /// `~/.codex/auth.json` if the subprocess fails to spawn for any
    /// reason (very old CLIs, sandboxed exec, etc.).
    ///
    /// Replaces an earlier file-only check that broke in the bundled
    /// `.app` on some hosts — `is_ok_and(|m| m.is_file())` returned
    /// `false` even when the file existed and was readable from a
    /// regular shell, which left the user stuck in `LoginRequired`
    /// after a successful `codex login`.
    pub async fn is_logged_in() -> bool {
        let Some(bin) = detect_cached() else {
            return false;
        };
        let probe = Command::new(&bin)
            .args(["login", "status"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        match timeout(Duration::from_secs(10), probe).await {
            Ok(Ok(out)) => out.status.success(),
            // Subprocess spawn / wait errored — fall back to the
            // file existence check so we don't regress hosts where
            // the probe is the broken path.
            _ => codex_auth_path()
                .map(|p| std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() > 0))
                .unwrap_or(false),
        }
    }
}

/// Three states for the memoised `which::which` result. Mirror of
/// [`crate::claude_cli`]; see that module for rationale.
enum BinaryCache {
    Unprobed,
    Missing,
    Found(PathBuf),
}

static CACHED: RwLock<BinaryCache> = RwLock::new(BinaryCache::Unprobed);

fn detect_cached() -> Option<PathBuf> {
    if let Ok(guard) = CACHED.read() {
        match &*guard {
            BinaryCache::Unprobed => {}
            BinaryCache::Missing => return None,
            BinaryCache::Found(p) => return Some(p.clone()),
        }
    }
    let resolved = which::which("codex").ok();
    if let Ok(mut guard) = CACHED.write() {
        *guard = match &resolved {
            Some(p) => BinaryCache::Found(p.clone()),
            None => BinaryCache::Missing,
        };
    }
    resolved
}

fn codex_auth_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".codex").join("auth.json"))
}

#[async_trait]
impl LlmProvider for CodexCliProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // Codex's flagship models advertise 400K context. We
            // report the floor of what callers can plan around.
            max_context_tokens: 400_000,
            // `exec` supports `--output-schema`, but we are not
            // wiring that today; flat JSON-mode is the minimum bar.
            supports_json_mode: false,
            supports_streaming: false,
            // `-i` / image attachments are interactive-only per docs.
            vision: false,
            scope: ProviderScope::General,
        }
    }

    fn requires_network(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let Some(bin) = self.binary.as_ref() else {
            return Err(ProviderError::Unavailable(
                "`codex` not found on PATH. Install Codex CLI and run \
                 `codex login` to use this provider."
                    .into(),
            ));
        };

        let prompt = flatten_messages_for_codex(&req);

        let cwd = TempDir::new().map_err(|e| ProviderError::Other(format!("tempdir: {e}")))?;
        let last_msg_path = cwd.path().join("last-message.txt");

        let mut cmd = Command::new(bin);
        cmd.arg("exec")
            .arg("--sandbox")
            .arg("read-only")
            .arg("--skip-git-repo-check")
            .arg("--ephemeral")
            .arg("--ignore-user-config")
            .arg("--ignore-rules")
            .arg("--color")
            .arg("never")
            .arg("--output-last-message")
            .arg(&last_msg_path)
            .arg(prompt.as_str());

        cmd.current_dir(cwd.path());

        // Scrub env vars that could rewire the CLI silently. Keep
        // PATH and HOME (the latter is needed for ~/.codex/...
        // credential lookup).
        for (key, _) in std::env::vars() {
            if key.starts_with("CODEX_") || key.starts_with("OPENAI_") {
                cmd.env_remove(&key);
            }
        }

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| ProviderError::Other(format!("spawn codex: {e}")))?;

        let out = match timeout(COMPLETION_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(ProviderError::Other(format!("wait codex: {e}"))),
            Err(_) => {
                return Err(ProviderError::Other(format!(
                    "`codex exec` timed out after {} seconds",
                    COMPLETION_TIMEOUT.as_secs()
                )))
            }
        };

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            // Keep the scratch dir alive past the success/failure
            // branch — dropping it would delete `last_msg_path` and
            // the diagnostics we still need to read.
            drop(cwd);
            if looks_like_auth_failure(&stderr) {
                return Err(ProviderError::Auth);
            }
            return Err(ProviderError::Other(format!(
                "codex exec exited {}: {}",
                out.status
                    .code()
                    .map_or_else(|| "?".into(), |c| c.to_string()),
                stderr.trim()
            )));
        }

        // Codex writes the agent's final message to the path passed
        // via `--output-last-message`. Reading the file is cleaner
        // than parsing stdout, which is interleaved with the session
        // header (`workdir`, `model`, …), reasoning lines, and the
        // `tokens used` footer.
        let text = match tokio::fs::read_to_string(&last_msg_path).await {
            Ok(s) => s.trim().to_owned(),
            Err(e) => {
                drop(cwd);
                return Err(ProviderError::Other(format!(
                    "codex exec did not write last-message file: {e}"
                )));
            }
        };
        drop(cwd);

        if text.is_empty() {
            return Err(ProviderError::Other(
                "codex exec returned empty last-message file".into(),
            ));
        }

        Ok(CompletionResponse {
            text,
            usage: Usage::default(),
            model: DEFAULT_MODEL.to_owned(),
            web_citations: Vec::new(),
        })
    }
}

/// Roll the request's messages into a single prompt string. Codex
/// `exec` does not accept a system prompt flag, so any system content
/// is emitted ahead of the user content with an explicit separator.
fn flatten_messages_for_codex(req: &CompletionRequest) -> String {
    let mut system_parts: Vec<&str> = Vec::new();
    let mut convo: Vec<(MessageRole, &str)> = Vec::new();
    for m in &req.messages {
        match m.role {
            MessageRole::System => system_parts.push(m.content.as_str()),
            r => convo.push((r, m.content.as_str())),
        }
    }

    let system = system_parts.join("\n\n");

    let body = if convo.len() == 1 {
        convo[0].1.to_owned()
    } else {
        let mut buf = String::with_capacity(convo.iter().map(|(_, s)| s.len() + 16).sum());
        for (role, content) in &convo {
            let marker = match role {
                MessageRole::User => "## User",
                MessageRole::Assistant => "## Assistant",
                MessageRole::System => unreachable!("system filtered above"),
            };
            if !buf.is_empty() {
                buf.push_str("\n\n");
            }
            buf.push_str(marker);
            buf.push('\n');
            buf.push_str(content);
        }
        buf
    };

    if system.is_empty() {
        body
    } else {
        format!("## Instructions\n{system}\n\n## Request\n{body}")
    }
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("authentication required")
        || lower.contains("please log in")
        || lower.contains("missing api key")
        || lower.contains("invalid api key")
        || lower.contains("`codex login`")
        || lower.contains("auth.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    fn req(messages: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            messages,
            ..CompletionRequest::default()
        }
    }

    #[test]
    fn flatten_single_user_passes_through() {
        let r = req(vec![Message::user("hola")]);
        assert_eq!(flatten_messages_for_codex(&r), "hola");
    }

    #[test]
    fn flatten_prepends_system_with_separator() {
        let r = req(vec![Message::system("rules"), Message::user("question")]);
        let prompt = flatten_messages_for_codex(&r);
        assert!(prompt.starts_with("## Instructions\nrules"));
        assert!(prompt.contains("## Request\nquestion"));
    }

    #[test]
    fn flatten_multiturn_uses_role_markers() {
        let r = req(vec![
            Message::user("Q1"),
            Message::assistant("A1"),
            Message::user("Q2"),
        ]);
        let prompt = flatten_messages_for_codex(&r);
        assert!(prompt.contains("## User\nQ1"));
        assert!(prompt.contains("## Assistant\nA1"));
        assert!(prompt.contains("## User\nQ2"));
    }

    #[test]
    fn auth_failure_heuristic_catches_codex_phrasings() {
        assert!(looks_like_auth_failure(
            "Error: Not logged in. Run `codex login`."
        ));
        assert!(looks_like_auth_failure("missing api key"));
        assert!(looks_like_auth_failure("could not read auth.json"));
        assert!(!looks_like_auth_failure("rate limit exceeded"));
    }

    #[test]
    fn capabilities_are_general_scope() {
        let p = CodexCliProvider::new();
        assert_eq!(p.capabilities().scope, ProviderScope::General);
        assert!(!p.capabilities().vision);
    }
}
