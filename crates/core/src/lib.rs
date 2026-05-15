//! Shared foundations for the Conclave workspace.
//!
//! `conclave-core` provides the cross-cutting primitives every other crate in
//! the workspace relies on: a unified [`Error`] type, an on-disk
//! [`Config`](config::Config) loaded from the OS-standard config directory,
//! resolved application [`Paths`](paths::Paths), and a single entry point for
//! initializing [`tracing`] subscribers.
//!
//! This crate is intentionally minimal during Phase 0: it carves the shape of
//! the API surface that the rest of the system (providers, RAG, deident, CLI)
//! will build on.

pub mod config;
pub mod error;
pub mod logging;
pub mod paths;

pub use config::{Config, GeneralConfig, LogFormat, ProvidersConfig, RagConfig};
pub use error::{Error, Result};
pub use paths::Paths;

/// Application identifier used to derive OS-standard directory paths.
pub const APP_QUALIFIER: &str = "dev";
/// Application organization used to derive OS-standard directory paths.
pub const APP_ORGANIZATION: &str = "Conclave";
/// Application name used to derive OS-standard directory paths.
pub const APP_NAME: &str = "conclave";

/// Medical-use disclaimer surfaced by every Conclave entry point.
///
/// Conclave is a research and decision-support tool. It does not make clinical
/// decisions and must not be used as a substitute for qualified medical
/// judgement.
pub const MEDICAL_DISCLAIMER: &str = "\
Conclave is an experimental clinical decision-support assistant. It is NOT a \
medical device and does NOT replace the judgement of a qualified clinician. \
Outputs may be incomplete, biased, or wrong. Always validate any suggestion \
against primary sources and institutional protocols before acting on it.";
