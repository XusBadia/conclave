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
//! - Model selection: we pass neither `--model` nor the user's
//!   `~/.codex/config.toml` (we run with `--ignore-user-config`), so
//!   the model is whatever the installed `codex` binary picks as its
//!   built-in default. The id reported to the UI ([`DEFAULT_MODEL`],
//!   `gpt-5.5`) is therefore a nominal label, not a binding request —
//!   it tracks Codex's default at the time of writing but can drift
//!   from the real model if a future CLI release changes that default.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::timeout;

use crate::cli_local::{
    binary_stats, stderr_excerpt, tracked_env_keys_present, ProbeDetails, PROBE_TIMEOUT,
};
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
///
/// Sized for the deliberative finalize phase (large prompt + full
/// verdict JSON, no enforceable output-token cap through the CLI) —
/// the same reasoning as the claude-cli provider, where 180 s was
/// measured too tight. A wedge detector, not a latency budget.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);

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

    /// `true` when the user has an active Codex session. Thin bool
    /// wrapper around [`Self::probe_login_detailed`] for callers
    /// (`list_providers`) that don't need the diagnostic payload.
    pub async fn is_logged_in() -> bool {
        Self::probe_login_detailed().await.logged_in
    }

    /// Probe `codex login status` and assemble a [`ProbeDetails`] for
    /// the Settings panel.
    ///
    /// Decision flow:
    /// 1. If `which::which("codex")` returned `None`, return the
    ///    `unresolved_binary()` snapshot — there's nothing to probe.
    /// 2. Run `codex login status` with stdio nulled, capturing
    ///    stderr to surface back to the user.
    /// 3. Trust an exit-0 outcome as logged in.
    /// 4. **Otherwise consult `~/.codex/auth.json`.** This is the key
    ///    change vs. the previous version, which only fell back when
    ///    the subprocess failed to *spawn*. Empirically the probe
    ///    exits non-zero from the `.app` even when the user is fully
    ///    logged in via the same `auth.json` the subprocess reads, so
    ///    treat the artifact as authoritative whenever the probe
    ///    isn't a clean success. `codex logout` deletes `auth.json`,
    ///    so this can't produce a false positive after logout.
    pub async fn probe_login_detailed() -> ProbeDetails {
        let Some(bin) = detect_cached() else {
            return ProbeDetails::unresolved_binary();
        };
        let (binary_mtime, binary_size) = binary_stats(&bin);
        let env_keys_seen = tracked_env_keys_present();

        let started = Instant::now();
        let probe = Command::new(&bin)
            .args(["login", "status"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output();
        let outcome = timeout(PROBE_TIMEOUT, probe).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut details = ProbeDetails {
            logged_in: false,
            command: Some("codex login status".to_owned()),
            exit_code: None,
            stderr_excerpt: String::new(),
            duration_ms: Some(duration_ms),
            timed_out: false,
            fallback_used: None,
            env_keys_seen,
            binary_mtime,
            binary_size,
        };

        match outcome {
            Ok(Ok(out)) => {
                details.exit_code = out.status.code();
                details.stderr_excerpt = stderr_excerpt(&out.stderr);
                if out.status.success() {
                    details.logged_in = true;
                } else if codex_auth_artifact_present() {
                    details.logged_in = true;
                    details.fallback_used = Some("~/.codex/auth.json".to_owned());
                }
            }
            Ok(Err(e)) => {
                details.stderr_excerpt = format!("spawn error: {e}");
                if codex_auth_artifact_present() {
                    details.logged_in = true;
                    details.fallback_used = Some("~/.codex/auth.json".to_owned());
                }
            }
            Err(_) => {
                details.timed_out = true;
                if codex_auth_artifact_present() {
                    details.logged_in = true;
                    details.fallback_used = Some("~/.codex/auth.json".to_owned());
                }
            }
        }

        details
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

/// `true` when `~/.codex/auth.json` exists and is non-empty. The Codex
/// CLI writes this file on successful `codex login` and deletes it on
/// `codex logout`, so it doubles as the persistent "is logged in?"
/// signal when the subprocess probe is unavailable or misbehaving.
fn codex_auth_artifact_present() -> bool {
    codex_auth_path()
        .map(|p| std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() > 0))
        .unwrap_or(false)
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

    /// `cargo test` runs tests in parallel by default; `HOME` is
    /// process-wide, so manipulating it from multiple threads races.
    /// We serialise the three cases through one test guarded by a
    /// mutex (covers env-poisoning if any previous test panicked).
    #[test]
    fn codex_auth_artifact_recognises_only_non_empty_file() {
        use std::sync::Mutex;
        static HOME_LOCK: Mutex<()> = Mutex::new(());
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let original = std::env::var_os("HOME");

        // (a) HOME unset → helper returns false.
        std::env::remove_var("HOME");
        assert!(!codex_auth_artifact_present(), "no HOME → not present");

        // (b) HOME points at a dir with a non-empty auth.json → true.
        let tmp_full = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp_full.path());
        std::fs::create_dir_all(tmp_full.path().join(".codex")).unwrap();
        std::fs::write(tmp_full.path().join(".codex/auth.json"), "{\"x\":1}").unwrap();
        assert!(
            codex_auth_artifact_present(),
            "non-empty auth.json → present"
        );

        // (c) HOME points at a dir with an empty auth.json → false.
        let tmp_empty = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp_empty.path());
        std::fs::create_dir_all(tmp_empty.path().join(".codex")).unwrap();
        std::fs::write(tmp_empty.path().join(".codex/auth.json"), "").unwrap();
        assert!(
            !codex_auth_artifact_present(),
            "empty auth.json → not present"
        );

        // Restore HOME so other tests see their original env.
        if let Some(h) = original {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
