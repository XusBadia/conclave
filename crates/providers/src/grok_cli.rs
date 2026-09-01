//! Grok CLI provider — runs the user's authenticated `grok` binary in
//! single-turn, headless mode.

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
use crate::{
    CompletionRequest, CompletionResponse, LlmProvider, MessageRole, ProviderCapabilities,
    ProviderError, ProviderScope, Usage,
};

pub const PROVIDER_ID: &str = "grok-cli";
pub const DEFAULT_MODEL: &str = "grok-4.6";
pub const DEFAULT_REASONING_EFFORT: &str = "high";
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct GrokCliProvider {
    binary: Option<PathBuf>,
    model: String,
    reasoning_effort: String,
}

impl std::fmt::Debug for GrokCliProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrokCliProvider")
            .field("binary", &self.binary)
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .finish()
    }
}

impl Default for GrokCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GrokCliProvider {
    pub fn new() -> Self {
        Self {
            binary: detect_cached(),
            model: DEFAULT_MODEL.to_owned(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.to_owned(),
        }
    }

    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    #[must_use]
    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = effort.into();
        self
    }

    pub fn binary_path() -> Option<PathBuf> {
        detect_cached()
    }

    pub fn is_installed() -> bool {
        detect_cached().is_some()
    }

    pub fn refresh_binary_cache() {
        if let Ok(mut guard) = CACHED.write() {
            *guard = BinaryCache::Unprobed;
        }
    }

    pub async fn is_logged_in() -> bool {
        Self::probe_login_detailed().await.logged_in
    }

    /// Grok currently has no `login status` subcommand. `grok models` is a
    /// cheap authenticated request and exits successfully only when the CLI
    /// can use the current session. The on-disk auth artifact is the same
    /// fallback used by the other CLI adapters when GUI-launched processes
    /// cannot complete their probe.
    pub async fn probe_login_detailed() -> ProbeDetails {
        let Some(bin) = detect_cached() else {
            return ProbeDetails::unresolved_binary();
        };
        let (binary_mtime, binary_size) = binary_stats(&bin);
        let env_keys_seen = tracked_env_keys_present();
        let started = Instant::now();
        let probe = Command::new(&bin)
            .arg("models")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output();
        let outcome = timeout(PROBE_TIMEOUT, probe).await;
        let mut details = ProbeDetails {
            logged_in: false,
            command: Some("grok models".to_owned()),
            exit_code: None,
            stderr_excerpt: String::new(),
            duration_ms: Some(started.elapsed().as_millis() as u64),
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
                details.logged_in = out.status.success();
            }
            Ok(Err(e)) => details.stderr_excerpt = format!("spawn error: {e}"),
            Err(_) => details.timed_out = true,
        }
        if !details.logged_in && grok_auth_artifact_present() {
            details.logged_in = true;
            details.fallback_used = Some("~/.grok/auth.json".to_owned());
        }
        details
    }
}

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
            BinaryCache::Found(path) => return Some(path.clone()),
        }
    }
    let resolved = which::which("grok").ok();
    if let Ok(mut guard) = CACHED.write() {
        *guard = match &resolved {
            Some(path) => BinaryCache::Found(path.clone()),
            None => BinaryCache::Missing,
        };
    }
    resolved
}

fn grok_auth_artifact_present() -> bool {
    std::env::var_os("HOME")
        .map(|home| Path::new(&home).join(".grok").join("auth.json"))
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|meta| meta.is_file() && meta.len() > 0)
}

#[async_trait]
impl LlmProvider for GrokCliProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            max_context_tokens: 256_000,
            supports_json_mode: false,
            supports_streaming: false,
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
                "`grok` not found on PATH. Install Grok CLI and run `grok login`.".into(),
            ));
        };
        let (system, prompt) = flatten_messages(&req);
        let model = if req.model.is_empty() {
            self.model.as_str()
        } else {
            req.model.as_str()
        };
        let cwd = TempDir::new().map_err(|e| ProviderError::Other(format!("tempdir: {e}")))?;
        let mut cmd = Command::new(bin);
        cmd.args([
            "--single",
            prompt.as_str(),
            "--output-format",
            "plain",
            "--model",
            model,
            "--reasoning-effort",
            self.reasoning_effort.as_str(),
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--no-subagents",
            "--disable-web-search",
            "--verbatim",
            "--cwd",
        ])
        .arg(cwd.path());
        if !system.is_empty() {
            cmd.arg("--system-prompt-override").arg(system);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("GROK_") || key.starts_with("XAI_") {
                cmd.env_remove(key);
            }
        }
        cmd.current_dir(cwd.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| ProviderError::Other(format!("spawn grok: {e}")))?;
        let out = match timeout(COMPLETION_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => return Err(ProviderError::Other(format!("wait grok: {e}"))),
            Err(_) => {
                return Err(ProviderError::Other(format!(
                    "`grok --single` timed out after {} seconds",
                    COMPLETION_TIMEOUT.as_secs()
                )))
            }
        };
        if !out.status.success() {
            let error = stderr_excerpt(&out.stderr);
            let lower = error.to_lowercase();
            if lower.contains("log in") || lower.contains("authentication") || lower.contains("401")
            {
                return Err(ProviderError::Auth);
            }
            return Err(ProviderError::Other(format!(
                "grok exited {}: {}",
                out.status
                    .code()
                    .map_or_else(|| "?".into(), |c| c.to_string()),
                error
            )));
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if text.is_empty() {
            return Err(ProviderError::Other(
                "grok returned an empty response".into(),
            ));
        }
        Ok(CompletionResponse {
            text,
            usage: Usage::default(),
            model: model.to_owned(),
            web_citations: Vec::new(),
        })
    }
}

fn flatten_messages(req: &CompletionRequest) -> (String, String) {
    let system = req
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = req
        .messages
        .iter()
        .filter(|message| message.role != MessageRole::System)
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => unreachable!(),
            };
            format!("## {role}\n{}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (system, prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    #[test]
    fn flatten_separates_system_and_conversation() {
        let req = CompletionRequest {
            messages: vec![Message::system("rules"), Message::user("question")],
            ..CompletionRequest::default()
        };
        let (system, prompt) = flatten_messages(&req);
        assert_eq!(system, "rules");
        assert_eq!(prompt, "## User\nquestion");
    }

    #[test]
    fn defaults_match_current_cli_models() {
        let provider = GrokCliProvider::new();
        assert_eq!(provider.model, "grok-4.6");
        assert_eq!(provider.reasoning_effort, "high");
    }
}
