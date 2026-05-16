//! `conclave-cli search` — vector-search the active workspace.

use anyhow::Result;
use clap::Args;

use super::CommandContext;

/// Arguments for the `search` subcommand.
#[derive(Debug, Args)]
pub(crate) struct SearchArgs {
    /// Query text (will be embedded and ANN-searched).
    query: String,
    /// Number of top hits to return.
    #[arg(long, short, default_value_t = 8)]
    k: usize,
}

pub(crate) async fn run(ctx: &CommandContext, args: SearchArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let embedder = ctx.default_embedder();
    let repo = ctx.open_repository(&workspace, &embedder).await?;

    let embedder_for_query = embedder.clone();
    let query_text = args.query.clone();
    let vectors =
        tokio::task::spawn_blocking(move || embedder_for_query.embed(&[query_text])).await??;
    let Some(query_vec) = vectors.into_iter().next() else {
        anyhow::bail!("embedder returned no vectors for the query");
    };

    let hits = repo.search(&query_vec, args.k).await?;
    if hits.is_empty() {
        println!("(no results — workspace `{}` may be empty)", workspace.id);
        return Ok(());
    }

    for (i, h) in hits.iter().enumerate() {
        let snippet: String = h.text.chars().take(200).collect();
        println!(
            "{:>2}. distance={:.4}  doc={}  chunk={}",
            i + 1,
            h.distance,
            h.document_id,
            h.chunk_id,
        );
        println!("    {snippet}");
    }
    Ok(())
}
