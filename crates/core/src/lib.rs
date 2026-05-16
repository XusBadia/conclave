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
pub mod workspace;

pub use config::{Config, GeneralConfig, LogFormat, ProvidersConfig, RagConfig};
pub use error::{Error, Result};
pub use paths::Paths;
pub use workspace::{Workspace, WorkspaceManager};

/// Application identifier used to derive OS-standard directory paths.
pub const APP_QUALIFIER: &str = "dev";
/// Application organization used to derive OS-standard directory paths.
pub const APP_ORGANIZATION: &str = "Conclave";
/// Application name used to derive OS-standard directory paths.
pub const APP_NAME: &str = "conclave";

/// Medical-use disclaimer surfaced by every Conclave entry point — English.
///
/// Conclave is a research and decision-support tool. It does not make clinical
/// decisions and must not be used as a substitute for qualified medical
/// judgement.
pub const MEDICAL_DISCLAIMER_EN: &str = "\
Conclave is an experimental clinical decision-support assistant. It is NOT a \
medical device and does NOT replace the judgement of a qualified clinician. \
Outputs may be incomplete, biased, or wrong. Always validate any suggestion \
against primary sources and institutional protocols before acting on it.";

/// Spanish translation of the medical-use disclaimer.
///
/// Used by the desktop frontend when the active UI locale is `es`. Keeping
/// both versions in the core crate ensures the text never drifts between
/// the CLI (English by default) and the GUI (locale-aware).
pub const MEDICAL_DISCLAIMER_ES: &str = "\
Conclave es un asistente experimental de soporte a la decisión clínica. NO es \
un dispositivo médico y NO sustituye el criterio de un profesional sanitario \
cualificado. Las respuestas pueden ser incompletas, sesgadas o erróneas. \
Verifica siempre cualquier sugerencia frente a las fuentes primarias y los \
protocolos institucionales antes de actuar.";

/// Backwards-compatible alias pointing at the English disclaimer.
///
/// Existing call sites (CLI, server logs, tests) keep working without changes.
/// New code paths that need a locale-aware string should pick between
/// [`MEDICAL_DISCLAIMER_EN`] and [`MEDICAL_DISCLAIMER_ES`] explicitly.
pub const MEDICAL_DISCLAIMER: &str = MEDICAL_DISCLAIMER_EN;
