//! `conclave-cli providers` — inspect configured LLM providers.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::CommandContext;

/// Arguments for the `providers` subcommand.
#[derive(Debug, Args)]
pub(crate) struct ProvidersArgs {
    #[command(subcommand)]
    pub(crate) action: ProvidersAction,
}

/// Sub-actions exposed by `providers`.
#[derive(Debug, Subcommand)]
pub(crate) enum ProvidersAction {
    /// List configured providers.
    List,
    /// Test connectivity with a specific provider.
    Test {
        /// Provider identifier (matches `providers.default` or a configured entry).
        #[arg(value_name = "ID")]
        id: String,
    },
}

/// Execute the `providers` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: ProvidersArgs) -> Result<()> {
    match args.action {
        ProvidersAction::List => {
            let default = ctx.config.providers.default.as_deref().unwrap_or("<unset>");
            println!("providers (default: {default}):");
            println!("  (phase 0) no providers wired yet — see phase 2 — providers");
        }
        ProvidersAction::Test { id } => {
            tracing::info!(provider = id.as_str(), "provider test invoked");
            println!("providers test: id={id}");
            println!("(phase 0) provider testing lands in phase 2");
        }
    }
    Ok(())
}
