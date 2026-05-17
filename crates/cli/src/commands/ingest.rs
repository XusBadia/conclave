//! `conclave-cli ingest` — push documents into the knowledge base.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Args;

use conclave_rag::{IngestionEvent, IngestionPipeline, SkipReason};

use super::CommandContext;

/// Arguments for the `ingest` subcommand.
#[derive(Debug, Args)]
pub(crate) struct IngestArgs {
    /// Path to a file or directory to ingest.
    #[arg(value_name = "PATH")]
    pub(crate) path: PathBuf,

    /// Workspace to ingest into (overrides the global --workspace flag and
    /// `config.general.default_workspace`).
    #[arg(long, value_name = "NAME")]
    pub(crate) workspace: Option<String>,

    /// Print elapsed time on completion.
    #[arg(long)]
    pub(crate) time: bool,
}

/// Execute the `ingest` subcommand.
pub(crate) async fn run(ctx: &CommandContext, args: IngestArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(args.workspace.as_deref())?;
    tracing::info!(
        path = %args.path.display(),
        workspace = %workspace.id,
        "ingest invoked"
    );

    let embedder = ctx.default_embedder();
    let repo = ctx.open_repository(&workspace, &embedder).await?;
    let chunk_params = ctx.chunk_params()?;
    let pipeline = IngestionPipeline::new(embedder, repo, chunk_params)?;

    let start = Instant::now();
    let report = pipeline
        .ingest_path(&args.path, |event| match event {
            IngestionEvent::Starting(p) => {
                println!("→ {}", p.display());
            }
            IngestionEvent::Ingested { path, record } => {
                println!(
                    "  ✓ {} → {} ({:?})",
                    path.display(),
                    record.id,
                    record.status
                );
            }
            IngestionEvent::Skipped { path, reason } => {
                let label = match reason {
                    SkipReason::UnsupportedType => "unsupported type",
                    SkipReason::NeedsOcr => "needs OCR (skip — feature `ocr` not active)",
                };
                println!("  · {} skipped: {label}", path.display());
            }
            IngestionEvent::Failed { path, error } => {
                println!("  ✗ {} failed: {error}", path.display());
            }
            // Granular progress events are GUI-only (we forward them over a
            // Tauri channel). The CLI keeps the existing per-file output.
            IngestionEvent::Progress { .. } => {}
        })
        .await?;
    let elapsed = start.elapsed();

    println!(
        "\ningest summary — workspace `{}`: {} ingested, {} skipped, {} failed",
        workspace.id,
        report.ingested.len(),
        report.skipped.len(),
        report.failed.len(),
    );
    if args.time {
        println!("elapsed: {elapsed:.2?}");
    }
    if !report.failed.is_empty() {
        anyhow::bail!("{} document(s) failed to ingest", report.failed.len());
    }
    Ok(())
}
