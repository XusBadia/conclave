//! Claude Code CLI provider — proxies completions through the user's
//! locally installed `claude` binary in non-interactive print mode.
//!
//! ## Why this exists
//!
//! Anthropic does not currently publish a public OAuth client id for
//! third-party apps that want to use a Claude subscription, and they
//! have actively restricted third-party tools that reuse the Claude
//! Code CLI's client id from outside Claude Code itself.
//!
//! `claude -p` (non-interactive print mode) is the vendor-documented
//! programmatic entry point and is paired with `claude setup-token`
//! ("long-lived OAuth token for CI and scripts — requires a Claude
//! subscription"). Asking the user to drive their inference through
//! their own locally installed and locally authenticated `claude`
//! binary is therefore the only **vendor-sanctioned** way for Conclave
//! to talk to a Claude subscription.
//!
//! ## How it works
//!
//! - At construction time we resolve `claude` in `$PATH`. Missing →
//!   the provider self-reports as unavailable; we never panic or error
//!   at the registry layer.
//! - `complete()` spawns:
//!   ```text
//!   claude -p
//!     --output-format json
//!     --max-turns 1
//!     --tools ""
//!     --no-session-persistence
//!     --disable-slash-commands
//!     --setting-sources project,local
//!     --model claude-sonnet-4-6
//!     --append-system-prompt-file <tempfile>
//!     [--json-schema '...']
//!     "<flattened conversation>"
//!   ```
//! - Why **not** `--bare`: bare mode skips keychain reads and forces
//!   auth via `ANTHROPIC_API_KEY` or `--settings`, which would defeat
//!   the entire reason this provider exists (using the user's Claude
//!   Pro/Max subscription via their already-logged-in CLI). We replace
//!   `--bare`'s isolation guarantees with a combination of:
//!   - `--tools ""` + `--max-turns 1` — single-turn, no tool calls.
//!   - `--no-session-persistence` — no transcript on disk.
//!   - `--disable-slash-commands` — skips skills and slash commands.
//!   - `--setting-sources project,local` — skips user-level settings
//!     (`~/.claude/settings.json`) so the user's personal config
//!     cannot rewire behaviour silently.
//!   - `TempDir` CWD — prevents auto-discovery of project-level
//!     `CLAUDE.md`, `AGENTS.md`, or `.claude/` folders from the host
//!     application directory.
//!
//!   This leaves `~/.claude/CLAUDE.md` (user memory) potentially
//!   loaded. For clinical use that risk is acceptable because the
//!   user's global memory is typically their own coding preferences
//!   rather than anything that would mislead a clinical reasoning
//!   prompt. Documented here so future hardening can be targeted.
//!
//! ## Limitations
//!
//! - Token counts: `claude -p --output-format json` does not currently
//!   surface `input_tokens` / `output_tokens` in a documented field, so
//!   [`Usage`] is reported as zeros. The cost field (`total_cost_usd`)
//!   is captured into the model echo when present, for future
//!   telemetry, but is not promoted into `Usage`.
//! - Vision: `-p` mode does not document image attachment support, so
//!   `capabilities().vision` is `false` and any
//!   [`ImageInput`](crate::types::ImageInput)s on the request are
//!   silently dropped.
//! - Web citations: not surfaced through the CLI's JSON output.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::timeout;

use crate::cli_local::{
    binary_stats, stderr_excerpt, tracked_env_keys_present, ProbeDetails, FALLBACK_TIMEOUT,
    PROBE_TIMEOUT,
};
use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, MessageRole, ProviderCapabilities, ProviderScope, Usage,
};
use crate::LlmProvider;

/// Stable model id we pin requests to.
///
/// The Claude Code CLI accepts the version-less form
/// (`claude-sonnet-4-6`) and resolves it to the most recent Sonnet 4.6
/// release at call time — matching the alias behaviour of the official
/// `--model sonnet` shortcut without locking us to "whatever Anthropic
/// decides Sonnet is right now".
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// Hard ceiling on a single completion. The CLI itself has no built-in
/// timeout for `-p`, so we enforce one here to avoid wedged batches.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(180);

/// Stable id used by the registry, `ProviderInfo.id`, and the keychain
/// scope (no keychain entry today, but reserved).
pub const PROVIDER_ID: &str = "claude-cli";

/// Provider that proxies a completion request to the local `claude`
/// binary in `--bare -p` mode.
#[derive(Clone)]
pub struct ClaudeCliProvider {
    /// Pre-resolved path to the `claude` binary, when present on
    /// `$PATH`. Resolved lazily and cached; `None` means the binary
    /// could not be found at construction time.
    binary: Option<PathBuf>,
    /// Model id passed via `--model`.
    model: String,
}

impl std::fmt::Debug for ClaudeCliProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCliProvider")
            .field("binary", &self.binary)
            .field("model", &self.model)
            .finish()
    }
}

impl Default for ClaudeCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeCliProvider {
    /// Construct a provider with the cached lookup of `claude` in `$PATH`.
    /// Cheap: the lookup is memoised process-wide.
    pub fn new() -> Self {
        Self {
            binary: detect_cached(),
            model: DEFAULT_MODEL.to_owned(),
        }
    }

    /// Override the model id passed via `--model`.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Process-wide cached resolution of the `claude` binary path. We
    /// memoise on first hit because `which::which` walks `$PATH` and
    /// can stat directories repeatedly; the binary's location does not
    /// change while Conclave is running unless the user installs the
    /// CLI in another window and calls [`refresh_binary_cache`].
    pub fn binary_path() -> Option<PathBuf> {
        detect_cached()
    }

    /// `true` when `claude` is present on `$PATH`.
    pub fn is_installed() -> bool {
        detect_cached().is_some()
    }

    /// Invalidate the memoised binary path so the next call re-walks
    /// `$PATH`. Used after the user installs the CLI in a terminal
    /// while Conclave is running — the Settings panel calls this via
    /// the `redetect_cli_binaries` Tauri command before re-probing.
    pub fn refresh_binary_cache() {
        if let Ok(mut guard) = CACHED.write() {
            *guard = BinaryCache::Unprobed;
        }
    }

    /// `true` when the user has an active Claude session. Thin bool
    /// wrapper around [`Self::probe_login_detailed`] for callers
    /// (`list_providers`) that don't need the diagnostic payload.
    pub async fn is_logged_in() -> bool {
        Self::probe_login_detailed().await.logged_in
    }

    /// Probe `claude auth status` and assemble a [`ProbeDetails`] for
    /// the Settings panel.
    ///
    /// Decision flow:
    /// 1. Binary missing → `unresolved_binary()` snapshot.
    /// 2. Run `claude auth status` with `HOME` / `USER` / `LOGNAME`
    ///    re-set defensively from the parent process. The Tauri parent
    ///    *should* be propagating them via the usual launchd inherit,
    ///    but we explicitly forward them so a stripped child env can't
    ///    silently make the CLI miss its credentials.
    /// 3. Exit 0 → logged in, no fallback.
    /// 4. Non-zero / timeout / spawn error → try the macOS Keychain
    ///    item directly with `/usr/bin/security find-generic-password`
    ///    (no `-w` / `-g`, so it returns metadata only and never
    ///    triggers a Keychain ACL prompt). Exit 0 from that command
    ///    means the credential record exists → treat as logged in.
    /// 5. Last resort: check `~/.claude/.credentials.json` for legacy
    ///    installs that haven't moved to the Keychain yet.
    ///
    /// We never read the secret value — only check for presence —
    /// which keeps the Keychain ACL silent.
    pub async fn probe_login_detailed() -> ProbeDetails {
        let Some(bin) = detect_cached() else {
            return ProbeDetails::unresolved_binary();
        };
        let (binary_mtime, binary_size) = binary_stats(&bin);
        let env_keys_seen = tracked_env_keys_present();

        let started = Instant::now();
        let mut probe_cmd = Command::new(&bin);
        probe_cmd
            .args(["auth", "status"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Defensive env hygiene: re-forward the keychain-context vars
        // that some CLIs require to read their stored credentials. If
        // the parent already has them set, this is a no-op; if launchd
        // or a Tauri plugin scrubbed any, we restore them.
        if let Some(home) = std::env::var_os("HOME") {
            probe_cmd.env("HOME", home);
        }
        if let Some(user) = std::env::var_os("USER") {
            probe_cmd.env("USER", user);
        }
        if let Some(logname) = std::env::var_os("LOGNAME") {
            probe_cmd.env("LOGNAME", logname);
        } else if let Some(user) = std::env::var_os("USER") {
            // Some launchd contexts set USER but not LOGNAME. Mirror
            // USER so any CLI that reads LOGNAME finds a value.
            probe_cmd.env("LOGNAME", user);
        }

        let outcome = timeout(PROBE_TIMEOUT, probe_cmd.output()).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut details = ProbeDetails {
            logged_in: false,
            command: Some("claude auth status".to_owned()),
            exit_code: None,
            stderr_excerpt: String::new(),
            duration_ms: Some(duration_ms),
            timed_out: false,
            fallback_used: None,
            env_keys_seen,
            binary_mtime,
            binary_size,
        };

        let probe_succeeded = match outcome {
            Ok(Ok(out)) => {
                details.exit_code = out.status.code();
                details.stderr_excerpt = stderr_excerpt(&out.stderr);
                out.status.success()
            }
            Ok(Err(e)) => {
                details.stderr_excerpt = format!("spawn error: {e}");
                false
            }
            Err(_) => {
                details.timed_out = true;
                false
            }
        };

        if probe_succeeded {
            details.logged_in = true;
            return details;
        }

        // Fallback 1: macOS Keychain. Exits 0 if the credential record
        // exists. We do NOT pass `-w` or `-g`, so no secret is read and
        // no ACL prompt fires — we just observe metadata presence.
        if cfg!(target_os = "macos") && keychain_credential_present().await {
            details.logged_in = true;
            details.fallback_used = Some("keychain".to_owned());
            return details;
        }

        // Fallback 2: legacy credentials file. Some hosts (pre-2025
        // installs that never migrated to the Keychain) still keep the
        // OAuth tokens here. Non-empty file is enough — we don't parse.
        if let Some(p) = claude_legacy_credentials_path() {
            if std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() > 0) {
                details.logged_in = true;
                details.fallback_used = Some("~/.claude/.credentials.json".to_owned());
                return details;
            }
        }

        details
    }
}

/// Three states for the memoised `which::which` result:
/// `Unprobed` (initial / after refresh) → re-walk PATH on next read,
/// `Missing` (probe ran, binary not found), `Found` (probe ran, has path).
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
    let resolved = which::which("claude").ok();
    if let Ok(mut guard) = CACHED.write() {
        *guard = match &resolved {
            Some(p) => BinaryCache::Found(p.clone()),
            None => BinaryCache::Missing,
        };
    }
    resolved
}

/// Legacy on-disk credentials path used by older Claude Code installs
/// before the Keychain migration. `None` when `HOME` is unset.
fn claude_legacy_credentials_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".claude").join(".credentials.json"))
}

/// `true` when the macOS Login Keychain has a generic-password record
/// for the service name Claude Code uses (`"Claude Code-credentials"`).
/// We invoke `/usr/bin/security` rather than linking the Security
/// framework directly — same effect, smaller binary, and the system
/// tool is universally available.
///
/// Critically: we do NOT pass `-w` (extract password to stdout) or
/// `-g` (extract to stderr). With neither flag the tool reads only the
/// item's metadata, which does not require ACL approval and never
/// triggers the "claude wants to access this item" dialog.
#[cfg(target_os = "macos")]
async fn keychain_credential_present() -> bool {
    let user = match std::env::var("USER") {
        Ok(u) if !u.is_empty() => u,
        _ => return false,
    };
    let probe = Command::new("/usr/bin/security")
        .arg("find-generic-password")
        .arg("-a")
        .arg(&user)
        .arg("-s")
        .arg("Claude Code-credentials")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    matches!(
        timeout(FALLBACK_TIMEOUT, probe).await,
        Ok(Ok(out)) if out.status.success()
    )
}

#[cfg(not(target_os = "macos"))]
async fn keychain_credential_present() -> bool {
    false
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 200_000,
            supports_json_mode: true,
            supports_streaming: false,
            // `-p` mode is not documented to accept image attachments;
            // we drop them silently rather than failing the request.
            vision: false,
            scope: ProviderScope::General,
        }
    }

    fn requires_network(&self) -> bool {
        // The CLI itself calls the Anthropic API.
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let Some(bin) = self.binary.as_ref() else {
            return Err(ProviderError::Unavailable(
                "`claude` not found on PATH. Install Claude Code and run \
                 `claude auth login` to use this provider."
                    .into(),
            ));
        };

        let (system_text, user_prompt) = flatten_messages(&req);

        // The CLI accepts `--append-system-prompt` inline, but a very
        // long system block (clinical guidelines, several KB) can
        // bump up against macOS's argv length limit if combined with
        // a long user prompt. Always route the system through a file
        // for predictable behaviour.
        let cwd = TempDir::new().map_err(|e| ProviderError::Other(format!("tempdir: {e}")))?;
        let system_path = cwd.path().join("system.txt");
        if !system_text.is_empty() {
            tokio::fs::write(&system_path, &system_text)
                .await
                .map_err(|e| ProviderError::Other(format!("write system: {e}")))?;
        }

        let model = if req.model.is_empty() {
            self.model.clone()
        } else {
            req.model.clone()
        };

        let mut cmd = Command::new(bin);
        cmd.args([
            "-p",
            "--output-format",
            "json",
            "--max-turns",
            "1",
            "--tools",
            "",
            "--no-session-persistence",
            "--disable-slash-commands",
            "--setting-sources",
            "project,local",
            "--model",
            model.as_str(),
        ]);
        if !system_text.is_empty() {
            cmd.arg("--append-system-prompt-file").arg(&system_path);
        }
        if let Some(schema) = &req.json_schema {
            cmd.arg("--json-schema").arg(schema.to_string());
        }
        cmd.arg(&user_prompt);

        // Scratch CWD so a stray CLAUDE.md / AGENTS.md / `.claude/`
        // folder in the host project cannot inject prompts into the
        // model. `--bare` already disables auto-discovery, but this is
        // a second line of defence.
        cmd.current_dir(cwd.path());

        // Scrub env vars that could rewire the CLI silently. We keep
        // PATH (needed for the binary's own runtime resolution) and
        // HOME (needed for `~/.claude/...` credential lookup). We
        // wipe every other CLAUDE_* / ANTHROPIC_* variable so test
        // harnesses or shell profiles cannot influence the run.
        for (key, _) in std::env::vars() {
            if key.starts_with("CLAUDE_")
                || key.starts_with("ANTHROPIC_")
                || key == "CLAUDE_CODE_SIMPLE"
            {
                cmd.env_remove(&key);
            }
        }

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| ProviderError::Other(format!("spawn claude: {e}")))?;

        // Tokio gives us `child.wait_with_output()` for the
        // collect-everything case. Wrap it in our timeout.
        let out = match timeout(COMPLETION_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(ProviderError::Other(format!("wait claude: {e}"))),
            Err(_) => {
                return Err(ProviderError::Other(format!(
                    "`claude -p` timed out after {} seconds",
                    COMPLETION_TIMEOUT.as_secs()
                )))
            }
        };

        // Keep the TempDir alive until after the process has fully
        // exited — dropping it earlier would delete the system file
        // mid-run. The explicit drop here makes the lifetime obvious.
        drop(cwd);

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            if looks_like_auth_failure(&stderr) {
                return Err(ProviderError::Auth);
            }
            return Err(ProviderError::Other(format!(
                "claude -p exited {}: {}",
                out.status
                    .code()
                    .map_or_else(|| "?".into(), |c| c.to_string()),
                stderr.trim()
            )));
        }

        let stdout = std::str::from_utf8(&out.stdout)
            .map_err(|_| ProviderError::Other("claude -p produced non-UTF8 stdout".into()))?;

        // Newer Claude Code versions occasionally emit a "session
        // started" preamble line before the JSON. Trim to the first
        // `{` so the deserialiser sees clean JSON.
        let json_start = stdout
            .find('{')
            .ok_or_else(|| ProviderError::Other("claude -p produced no JSON output".into()))?;
        let parsed: ClaudeCliResult = serde_json::from_str(&stdout[json_start..])
            .map_err(|e| ProviderError::Other(format!("claude JSON parse: {e}")))?;

        if parsed.is_error == Some(true) {
            // The CLI surfaces inferred-error responses with an
            // `is_error: true` flag. Treat as a provider error so
            // upstream code can fall through to the retry path.
            return Err(ProviderError::Other(
                parsed
                    .result
                    .unwrap_or_else(|| "claude reported error".into()),
            ));
        }

        let text = parsed
            .result
            .ok_or_else(|| ProviderError::Other("claude JSON missing `result`".into()))?;

        let echoed_model = parsed.model.unwrap_or(model);

        Ok(CompletionResponse {
            text,
            usage: Usage {
                input_tokens: parsed
                    .usage
                    .as_ref()
                    .and_then(|u| u.input_tokens)
                    .unwrap_or(0),
                output_tokens: parsed
                    .usage
                    .as_ref()
                    .and_then(|u| u.output_tokens)
                    .unwrap_or(0),
            },
            model: echoed_model,
            web_citations: Vec::new(),
        })
    }
}

/// Roll the request's messages into a `(system, user)` pair suitable
/// for `--append-system-prompt-file` + positional prompt.
///
/// - All `MessageRole::System` content is concatenated (in order) with
///   blank-line separators and returned as the system block.
/// - Non-system messages are flattened into a single user prompt with
///   `## User` / `## Assistant` markers per turn. When there is only
///   one user message, we hand it through verbatim so single-turn
///   completions do not pick up extra structure noise.
fn flatten_messages(req: &CompletionRequest) -> (String, String) {
    let mut system_parts: Vec<&str> = Vec::new();
    let mut convo: Vec<(MessageRole, &str)> = Vec::new();
    for m in &req.messages {
        match m.role {
            MessageRole::System => system_parts.push(m.content.as_str()),
            r => convo.push((r, m.content.as_str())),
        }
    }
    let system = system_parts.join("\n\n");

    let user = if convo.len() == 1 {
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
    (system, user)
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("authentication required")
        || lower.contains("please log in")
        || lower.contains("credentials")
        || lower.contains("`claude auth login`")
}

/// Schema for `claude -p --output-format json`. Fields we don't use are
/// deserialised but ignored; missing fields are tolerated where the
/// caller falls back gracefully.
#[derive(Debug, Deserialize)]
struct ClaudeCliResult {
    /// Final assistant text. Present on successful runs.
    #[serde(default)]
    result: Option<String>,
    /// Echoed model id, e.g. `claude-sonnet-4-6-20250929`. Optional;
    /// older CLI versions did not emit this.
    #[serde(default)]
    model: Option<String>,
    /// Token accounting, when present.
    #[serde(default)]
    usage: Option<ClaudeCliUsage>,
    /// Some error paths set this flag and use `result` as the human
    /// message instead of the assistant output.
    #[serde(default)]
    is_error: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ClaudeCliUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

// Unused fields the JSON emits are silently ignored by serde with the
// `#[serde(default)]` strategy above. We do not declare a denying
// `#[serde(deny_unknown_fields)]` because the CLI's output schema is
// still evolving.

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
    fn flatten_single_user_message_is_passed_through_verbatim() {
        let r = req(vec![Message::user("hola")]);
        let (sys, user) = flatten_messages(&r);
        assert!(sys.is_empty());
        assert_eq!(user, "hola");
    }

    #[test]
    fn flatten_concatenates_system_blocks_with_blank_lines() {
        let r = req(vec![
            Message::system("rule A"),
            Message::system("rule B"),
            Message::user("question"),
        ]);
        let (sys, user) = flatten_messages(&r);
        assert_eq!(sys, "rule A\n\nrule B");
        assert_eq!(user, "question");
    }

    #[test]
    fn flatten_multiturn_uses_role_markers() {
        let r = req(vec![
            Message::user("Q1"),
            Message::assistant("A1"),
            Message::user("Q2"),
        ]);
        let (_, user) = flatten_messages(&r);
        assert!(user.contains("## User\nQ1"));
        assert!(user.contains("## Assistant\nA1"));
        assert!(user.contains("## User\nQ2"));
    }

    #[test]
    fn auth_failure_heuristic_catches_common_phrasings() {
        assert!(looks_like_auth_failure(
            "Error: Not logged in. Run `claude auth login`."
        ));
        assert!(looks_like_auth_failure("authentication required"));
        assert!(looks_like_auth_failure("invalid or missing credentials"));
        assert!(!looks_like_auth_failure("model overloaded"));
    }

    #[test]
    fn capabilities_are_general_scope() {
        let p = ClaudeCliProvider::new();
        assert_eq!(p.capabilities().scope, ProviderScope::General);
        assert!(!p.capabilities().vision);
        assert!(p.capabilities().supports_json_mode);
    }

    #[test]
    fn parse_minimal_json_output() {
        let json = r#"{"result":"hi","model":"claude-sonnet-4-6-20250929"}"#;
        let parsed: ClaudeCliResult = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.result.unwrap(), "hi");
        assert_eq!(parsed.model.unwrap(), "claude-sonnet-4-6-20250929");
    }

    #[test]
    fn parse_with_usage_block() {
        let json = r#"{
            "result": "ok",
            "usage": {"input_tokens": 12, "output_tokens": 34},
            "total_cost_usd": 0.0042
        }"#;
        let parsed: ClaudeCliResult = serde_json::from_str(json).unwrap();
        let u = parsed.usage.unwrap();
        assert_eq!(u.input_tokens.unwrap(), 12);
        assert_eq!(u.output_tokens.unwrap(), 34);
    }

    #[test]
    fn legacy_credentials_path_uses_home() {
        // HOME is process-wide; guard against parallel test races.
        use std::sync::Mutex;
        static HOME_LOCK: Mutex<()> = Mutex::new(());
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let original = std::env::var_os("HOME");
        std::env::set_var("HOME", "/tmp/conclave-test-home");
        let p = claude_legacy_credentials_path().unwrap();
        assert_eq!(
            p.to_string_lossy(),
            "/tmp/conclave-test-home/.claude/.credentials.json"
        );
        if let Some(h) = original {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[tokio::test]
    async fn probe_details_for_missing_binary() {
        // Force the cache into Missing so probe_login_detailed short-circuits.
        if let Ok(mut guard) = CACHED.write() {
            *guard = BinaryCache::Missing;
        }
        let details = ClaudeCliProvider::probe_login_detailed().await;
        assert!(!details.logged_in);
        assert!(details.command.is_none());
        assert!(details.exit_code.is_none());
        // Restore Unprobed so other tests can re-detect from a clean state.
        if let Ok(mut guard) = CACHED.write() {
            *guard = BinaryCache::Unprobed;
        }
    }
}
