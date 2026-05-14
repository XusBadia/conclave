//! `conclave-cli ingest` — push documents into the knowledge base.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use conclave_rag::{
    ingest::{ingest_path, DocumentOutcome},
    ChunkParams, IngestRequest, KnowledgeStore,
};

use super::CommandContext;
use crate::embedder;

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

    /// Override the chunk size (characters). Defaults to `config.rag.chunk_size`.
    #[arg(long, value_name = "N")]
    pub(crate) chunk_size: Option<usize>,

    /// Override the chunk overlap (characters). Defaults to `config.rag.chunk_overlap`.
    #[arg(long, value_name = "N")]
    pub(crate) chunk_overlap: Option<usize>,
}

/// Execute the `ingest` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: IngestArgs) -> Result<()> {
    let workspace = args
        .workspace
        .as_deref()
        .unwrap_or(&ctx.config.general.default_workspace);
    let chunk_size = args.chunk_size.unwrap_or(ctx.config.rag.chunk_size);
    let overlap = args.chunk_overlap.unwrap_or(ctx.config.rag.chunk_overlap);
    let chunk = ChunkParams::new(chunk_size, overlap)?;

    let db = ctx.workspace_db(Some(workspace));
    let embedder = embedder::resolve(&ctx.paths, &ctx.config.knowledge)?;
    let mut store = KnowledgeStore::open(&db, ctx.config.knowledge.embedding_dim)?;

    tracing::info!(
        path = %args.path.display(),
        workspace,
        dry_run = args.dry_run,
        chunk_size,
        overlap,
        "ingest invoked"
    );

    let report = ingest_path(
        &mut store,
        embedder.as_ref(),
        &IngestRequest {
            root: &args.path,
            chunk,
            dry_run: args.dry_run,
        },
    )?;

    print_report(&report, args.dry_run);
    Ok(())
}

fn print_report(report: &conclave_rag::IngestReport, dry_run: bool) {
    let mode = if dry_run { "dry-run" } else { "ingest" };
    println!(
        "{mode}: visited={visited} inserted={inserted} replaced={replaced} \
         unchanged={unchanged} skipped={skipped} failed={failed}",
        visited = report.visited,
        inserted = report.inserted(),
        replaced = report.replaced(),
        unchanged = report.unchanged(),
        skipped = report.skipped(),
        failed = report.failed(),
    );
    for outcome in &report.outcomes {
        match outcome {
            DocumentOutcome::Inserted {
                path,
                format,
                chunks,
            } => println!(
                "  + {kind:8} {path} ({chunks} chunks)",
                kind = format.label(),
                path = path.display(),
            ),
            DocumentOutcome::Replaced {
                path,
                format,
                chunks,
            } => println!(
                "  ~ {kind:8} {path} ({chunks} chunks)",
                kind = format.label(),
                path = path.display(),
            ),
            DocumentOutcome::Unchanged { path } => {
                println!("  = unchanged {path}", path = path.display());
            }
            DocumentOutcome::Skipped { path, reason } => {
                println!("  - skipped  {path} ({reason})", path = path.display());
            }
            DocumentOutcome::Failed { path, error } => {
                eprintln!("  ! failed   {path}: {error}", path = path.display());
            }
        }
    }
}
