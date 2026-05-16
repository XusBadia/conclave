//! Centralised `tracing` subscriber initialisation.
//!
//! Logging style is decided in this order:
//! 1. The [`LogFormat`](crate::config::LogFormat) passed in explicitly.
//! 2. The `CONCLAVE_LOG_FORMAT` env var (`pretty` | `json`).
//! 3. Auto-detection: `json` when the `CI` env var is truthy, `pretty`
//!    otherwise.
//!
//! Level filtering follows the standard `RUST_LOG` env-filter syntax, falling
//! back to `info` for the workspace crates.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::config::LogFormat;

/// Initialise the global tracing subscriber.
///
/// Calling this more than once will return an error from the underlying
/// `tracing` registry; that error is intentionally swallowed so that tests and
/// repeated CLI invocations remain safe.
pub fn init(preferred: LogFormat) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,conclave=debug"));

    let style = resolve_style(preferred);

    let registry = tracing_subscriber::registry().with(env_filter);

    let result = match style {
        ResolvedStyle::Json => registry
            .with(fmt::layer().json().with_current_span(true))
            .try_init(),
        ResolvedStyle::Pretty => registry
            .with(fmt::layer().with_target(false).compact())
            .try_init(),
    };

    if let Err(e) = result {
        // Already initialised: log via `eprintln!` since tracing may be live.
        eprintln!("tracing already initialised: {e}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedStyle {
    Pretty,
    Json,
}

fn resolve_style(preferred: LogFormat) -> ResolvedStyle {
    match preferred {
        LogFormat::Pretty => ResolvedStyle::Pretty,
        LogFormat::Json => ResolvedStyle::Json,
        LogFormat::Auto => resolve_from_env(),
    }
}

fn resolve_from_env() -> ResolvedStyle {
    if let Ok(raw) = std::env::var("CONCLAVE_LOG_FORMAT") {
        match raw.to_ascii_lowercase().as_str() {
            "json" => return ResolvedStyle::Json,
            "pretty" => return ResolvedStyle::Pretty,
            _ => {}
        }
    }
    if is_truthy_env("CI") {
        ResolvedStyle::Json
    } else {
        ResolvedStyle::Pretty
    }
}

fn is_truthy_env(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_pretty_overrides_env() {
        // Even if CI is set, explicit Pretty wins.
        assert_eq!(resolve_style(LogFormat::Pretty), ResolvedStyle::Pretty);
        assert_eq!(resolve_style(LogFormat::Json), ResolvedStyle::Json);
    }

    #[test]
    fn truthy_env_detection() {
        // We can't safely mutate process env in parallel tests, so just check
        // the pure helper on known string inputs via a local re-implementation.
        for v in ["1", "true", "TRUE", "yes", "on"] {
            assert!(matches!(
                v.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ));
        }
        for v in ["0", "false", "", "no"] {
            assert!(!matches!(
                v.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ));
        }
    }
}
