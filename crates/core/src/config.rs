//! TOML-backed application configuration.
//!
//! The on-disk layout is a single `conclave.toml` file living under the
//! [`Paths::config_dir`](crate::paths::Paths::config_dir). It is loaded with
//! [`Config::load`] and persisted with [`Config::save`].

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

/// RAG pipeline configuration. Real defaults will be re-tuned in Phase 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RagConfig {
    /// Approximate characters per chunk.
    pub chunk_size: usize,
    /// Overlap, in characters, between adjacent chunks.
    pub chunk_overlap: usize,
    /// Top-K candidates to retrieve per query.
    pub top_k: usize,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1024,
            chunk_overlap: 128,
            top_k: 8,
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
