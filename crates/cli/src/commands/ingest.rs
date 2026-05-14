//! `conclave-cli ingest` — push documents into the knowledge base.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use super::CommandContext;

/// Arguments for the `ingest` subcommand.
#[derive(Debug, Args)]
pub(crate) struct IngestArgs {
    /// Path to a file or directory to ingest.
    #[arg(value_name = "PATH")]
    pub(crate) path: PathBuf,

    /// Workspace to ingest into (defaults to `config.general.default_workspace`).
    #[arg(long, value_name = "NAME")]
    pub(crate) workspace: Option<String>,

    /// Dry run: walk inputs and report what *would* be ingested.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

/// Execute the `ingest` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: IngestArgs) -> Result<()> {
    let workspace = args
        .workspace
        .as_deref()
        .unwrap_or(&ctx.config.general.default_workspace);
    tracing::info!(
        path = %args.path.display(),
        workspace,
        dry_run = args.dry_run,
        "ingest invoked"
    );
    println!(
        "ingest: target={path} workspace={workspace} dry_run={dry_run}",
        path = args.path.display(),
        workspace = workspace,
        dry_run = args.dry_run,
    );
    println!("(phase 0) ingestion pipeline lands in phase 1 — knowledge base");
    Ok(())
}
