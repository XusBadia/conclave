//! TOML-backed application configuration.
//!
//! The on-disk layout is a single `conclave.toml` file living under the
//! [`Paths::config_dir`](crate::paths::Paths::config_dir). It is loaded with
//! [`Config::load`] and persisted with [`Config::save`].

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Root configuration object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// General application settings.
    pub general: GeneralConfig,
    /// RAG pipeline tuning.
    pub rag: RagConfig,
    /// Privacy posture for clinical runs.
    pub privacy: PrivacyConfig,
    /// LLM provider routing and credentials (filled in Phase 2).
    pub providers: ProvidersConfig,
}

/// Application-wide settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralConfig {
    /// Name of the workspace selected on launch.
    pub default_workspace: String,
    /// Preferred log output format (`auto`, `pretty`, `json`).
    pub log_format: LogFormat,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_workspace: "default".to_owned(),
            log_format: LogFormat::Auto,
        }
    }
}

/// Preferred log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Pretty in TTY, JSON when `CI=true`.
    #[default]
    Auto,
    /// Always pretty, human-friendly output.
    Pretty,
    /// Always structured JSON, one event per line.
    Json,
}

/// RAG pipeline configuration. Sizes are measured in **tokens**
/// (`cl100k_base` via `tiktoken-rs`), not characters — the chunker is
/// sentence-aware and honours these budgets approximately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RagConfig {
    /// Target tokens per chunk (upper bound when a sentence fits).
    pub chunk_size: usize,
    /// Tokens of overlap shared with the previous chunk.
    pub chunk_overlap: usize,
    /// Top-K candidates to retrieve per query.
    pub top_k: usize,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            chunk_size: 700,
            chunk_overlap: 100,
            top_k: 8,
        }
    }
}

/// Privacy defaults applied before each case can override them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    /// `local_only`, `deid_cloud`, or `explicit_phi`.
    pub default_data_boundary: String,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            default_data_boundary: "deid_cloud".to_owned(),
        }
    }
}

/// Container for LLM provider configuration.
///
/// Concrete provider entries land in Phase 2; for now this serves as a stable
/// section in the on-disk TOML file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Identifier of the default provider used when none is specified.
    pub default: Option<String>,
    /// Provider ids the user has explicitly disconnected from inside
    /// Conclave even though the underlying credential still exists
    /// (e.g. the Claude / Codex CLI binary is still installed and
    /// logged in via its own credentials). Treated as
    /// `not_configured` by `list_providers` so the picker re-appears.
    /// Cleared when the user picks the same provider tile again.
    #[serde(default)]
    pub disabled_provider_ids: Vec<String>,
    /// Per-CLI-provider manual override. When the user has the CLI
    /// installed and logged in but our auto-detection can't confirm
    /// it (Keychain ACL quirks, launchd-vs-shell env divergence, …),
    /// the Settings panel exposes a "Marcar como conectado" button
    /// that flips this map entry to `true`. `list_providers` then
    /// treats the provider as `Ready` whenever the binary is on
    /// `$PATH`, regardless of what the login probe returned. The
    /// entry persists across restarts; the user clears it via the
    /// same panel.
    #[serde(default)]
    pub cli_local_overrides: HashMap<String, bool>,
}

impl Config {
    /// Load configuration from `path`, falling back to defaults when the file
    /// does not exist.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let cfg: Self = toml::from_str(&raw)?;
                cfg.validate()?;
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::io_at(path, e)),
        }
    }

    /// Persist configuration to `path`, creating parent directories on demand.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw).map_err(|e| Error::io_at(path, e))?;
        Ok(())
    }

    /// Validate cross-field invariants.
    pub fn validate(&self) -> Result<()> {
        if self.general.default_workspace.trim().is_empty() {
            return Err(Error::invalid_config(
                "general.default_workspace must not be empty",
            ));
        }
        if self.rag.chunk_size == 0 {
            return Err(Error::invalid_config("rag.chunk_size must be > 0"));
        }
        if self.rag.chunk_overlap >= self.rag.chunk_size {
            return Err(Error::invalid_config(
                "rag.chunk_overlap must be < rag.chunk_size",
            ));
        }
        if self.rag.top_k == 0 {
            return Err(Error::invalid_config("rag.top_k must be > 0"));
        }
        if !matches!(
            self.privacy.default_data_boundary.as_str(),
            "local_only" | "deid_cloud" | "explicit_phi"
        ) {
            return Err(Error::invalid_config(
                "privacy.default_data_boundary must be local_only, deid_cloud or explicit_phi",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let cfg = Config::default();
        let raw = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.toml");
        let cfg = Config::load(&missing).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/dir/conclave.toml");

        let mut cfg = Config::default();
        cfg.general.default_workspace = "tumor-board".to_owned();
        cfg.general.log_format = LogFormat::Json;
        cfg.rag.chunk_size = 2048;
        cfg.rag.chunk_overlap = 256;
        cfg.privacy.default_data_boundary = "local_only".to_owned();
        cfg.providers.default = Some("anthropic".to_owned());

        cfg.save(&path).unwrap();
        assert!(path.exists());

        let loaded = Config::load(&path).unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn validate_rejects_empty_workspace() {
        let mut cfg = Config::default();
        cfg.general.default_workspace = "   ".to_owned();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_overlap_ge_chunk() {
        let mut cfg = Config::default();
        cfg.rag.chunk_overlap = cfg.rag.chunk_size;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unknown_default_data_boundary() {
        let mut cfg = Config::default();
        cfg.privacy.default_data_boundary = "send_everything".to_owned();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn disabled_provider_ids_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("conclave.toml");

        let mut cfg = Config::default();
        cfg.providers.disabled_provider_ids = vec!["claude-cli".to_owned(), "codex-cli".to_owned()];

        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.providers.disabled_provider_ids,
            vec!["claude-cli".to_owned(), "codex-cli".to_owned()]
        );
    }

    #[test]
    fn disabled_provider_ids_defaults_to_empty_when_missing() {
        let raw = r#"
            [providers]
            default = "anthropic"
        "#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert!(cfg.providers.disabled_provider_ids.is_empty());
    }

    #[test]
    fn cli_local_overrides_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("conclave.toml");

        let mut cfg = Config::default();
        cfg.providers
            .cli_local_overrides
            .insert("claude-cli".to_owned(), true);
        cfg.providers
            .cli_local_overrides
            .insert("codex-cli".to_owned(), false);

        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.providers.cli_local_overrides.get("claude-cli"),
            Some(&true)
        );
        assert_eq!(
            loaded.providers.cli_local_overrides.get("codex-cli"),
            Some(&false)
        );
    }

    #[test]
    fn cli_local_overrides_defaults_to_empty_when_missing() {
        let raw = r#"
            [providers]
            default = "anthropic"
        "#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert!(cfg.providers.cli_local_overrides.is_empty());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let raw = r#"
            [general]
            default_workspace = "x"
            log_format = "auto"
            bogus = "field"
        "#;
        let err = toml::from_str::<Config>(raw).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }
}
