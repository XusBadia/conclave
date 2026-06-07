//! Shared types and helpers for the local-CLI provider login probes.
//!
//! Both [`crate::claude_cli`] and [`crate::codex_cli`] face the same
//! brittleness: the documented `... status` subcommand exits cleanly
//! from a Terminal shell but can return non-zero from inside the
//! bundled `.app` (launchd-spawned process tree, occasionally
//! different keychain ACL paths, etc.). We capture every signal we
//! can — exit code, stderr, duration, which env keys were available,
//! whether we resorted to an artifact fallback — and surface it to the
//! Settings panel so the next "no detecta" report ships with enough
//! detail to diagnose without a code change.

use std::time::Duration;

use serde::Serialize;

/// Subset of the process environment we care about reporting back to
/// the UI. We only ever expose *presence* of these keys, never their
/// values — `HOME` and `USER` are stable and `PATH` is already
/// surfaced separately in `CliDiagnostics`, but listing presence makes
/// launchd-vs-shell divergence obvious in one screenshot.
pub const TRACKED_ENV_KEYS: &[&str] = &["HOME", "USER", "LOGNAME", "PATH"];

/// Snapshot of what `is_logged_in` did and what it observed.
///
/// Every field is optional in the wire sense (the frontend renders the
/// non-empty bits and skips the rest) so we can extend it without
/// breaking the TypeScript bindings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProbeDetails {
    /// Final verdict the rest of the app acts on. `true` only when the
    /// CLI is actually usable.
    pub logged_in: bool,
    /// The probe command we ran, joined by spaces (e.g.
    /// `"claude auth status"`). `None` when the binary couldn't be
    /// resolved and no probe was attempted.
    pub command: Option<String>,
    /// Exit code from the probe subprocess. `None` if it timed out,
    /// failed to spawn, or the binary wasn't found.
    pub exit_code: Option<i32>,
    /// First 200 chars of stderr, lossy-decoded. Empty string when
    /// stderr was empty or the subprocess never ran.
    pub stderr_excerpt: String,
    /// Wall-clock duration the probe took. `None` when no probe ran.
    pub duration_ms: Option<u64>,
    /// `true` when we cut the probe off at the hard timeout.
    pub timed_out: bool,
    /// Tag describing which artifact (if any) we ended up trusting
    /// when the probe failed. `None` when the probe itself succeeded.
    /// Examples: `"~/.codex/auth.json"`, `"keychain"`,
    /// `"~/.claude/.credentials.json"`.
    pub fallback_used: Option<String>,
    /// Names (NOT values) of environment variables that were present
    /// when we spawned the probe. Lets the panel verify that
    /// `USER` / `LOGNAME` are propagating from launchd.
    pub env_keys_seen: Vec<String>,
    /// Mtime of the resolved CLI binary as a unix timestamp. Sanity
    /// check that the installed/dev binary swap actually happened.
    pub binary_mtime: Option<u64>,
    /// Size of the resolved CLI binary in bytes. Same purpose as
    /// `binary_mtime`.
    pub binary_size: Option<u64>,
}

impl ProbeDetails {
    /// Build the empty "binary not found" snapshot. Returned by both
    /// providers when `which::which` came back empty.
    pub fn unresolved_binary() -> Self {
        Self {
            logged_in: false,
            command: None,
            exit_code: None,
            stderr_excerpt: String::new(),
            duration_ms: None,
            timed_out: false,
            fallback_used: None,
            env_keys_seen: tracked_env_keys_present(),
            binary_mtime: None,
            binary_size: None,
        }
    }
}

/// Names of `TRACKED_ENV_KEYS` whose value is currently set (non-empty).
/// Lossless and pure — no values are read.
pub fn tracked_env_keys_present() -> Vec<String> {
    TRACKED_ENV_KEYS
        .iter()
        .filter(|k| std::env::var_os(k).map(|v| !v.is_empty()).unwrap_or(false))
        .map(|k| (*k).to_owned())
        .collect()
}

/// Inspect the resolved binary path for an mtime + size pair. Failures
/// (file deleted between detection and probe, fs error) collapse to
/// `(None, None)`.
pub fn binary_stats(path: &std::path::Path) -> (Option<u64>, Option<u64>) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return (None, None),
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let size = Some(meta.len());
    (mtime, size)
}

/// First 200 characters of `stderr` (utf8-lossy decoded), trimmed.
/// We cap to keep the JSON payload small and avoid surfacing a megabyte
/// of debug output if a CLI gets very chatty.
pub fn stderr_excerpt(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let trimmed = s.trim();
    if trimmed.chars().count() <= 200 {
        trimmed.to_owned()
    } else {
        trimmed.chars().take(200).collect::<String>() + "…"
    }
}

/// Standard timeout we apply to the documented status probe. Long
/// enough to absorb keychain prompts and slow ACL resolution, short
/// enough that the Settings panel can't stall the whole window on a
/// wedged CLI.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Shorter timeout for artifact / fallback probes (the macOS
/// `security` tool and the filesystem checks). These should be
/// instant — anything slower is almost certainly a hang.
pub const FALLBACK_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_excerpt_short_passthrough() {
        let s = stderr_excerpt(b"  hello world  ");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn stderr_excerpt_long_truncated_with_ellipsis() {
        let long: String = "a".repeat(500);
        let s = stderr_excerpt(long.as_bytes());
        assert_eq!(s.chars().count(), 201);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn unresolved_snapshot_has_no_probe_signal() {
        let p = ProbeDetails::unresolved_binary();
        assert!(!p.logged_in);
        assert!(p.command.is_none());
        assert!(p.exit_code.is_none());
        assert!(p.duration_ms.is_none());
        assert!(p.fallback_used.is_none());
    }

    #[test]
    fn tracked_env_keys_lists_only_present_keys() {
        // HOME is essentially guaranteed in every test environment.
        let keys = tracked_env_keys_present();
        assert!(
            keys.contains(&"HOME".to_owned()),
            "expected HOME in {keys:?}"
        );
    }
}
