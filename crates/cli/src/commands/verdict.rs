//! `conclave-cli verdict` — run the virtual committee on a clinical question.

use anyhow::Result;
use clap::Args;

use super::CommandContext;

/// Arguments for the `verdict` subcommand.
#[derive(Debug, Args)]
pub(crate) struct VerdictArgs {
    /// Clinical question or case description.
    #[arg(value_name = "QUESTION")]
    pub(crate) question: String,

    /// Workspace to draw context from (defaults to the configured one).
    #[arg(long, value_name = "NAME")]
    pub(crate) workspace: Option<String>,

    /// Provider identifier overriding the configured default.
    #[arg(long, value_name = "ID")]
    pub(crate) provider: Option<String>,
}

/// Execute the `verdict` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: VerdictArgs) -> Result<()> {
    let workspace = args
        .workspace
        .as_deref()
        .unwrap_or(&ctx.config.general.default_workspace);
    let provider = args
        .provider
        .as_deref()
        .or(ctx.config.providers.default.as_deref())
        .unwrap_or("<none configured>");
    tracing::info!(workspace, provider, "verdict invoked");
    println!("verdict: workspace={workspace} provider={provider}");
    println!("question: {q}", q = args.question);
    println!("(phase 0) committee orchestration lands in phase 4 — deliberation");
    Ok(())
}
