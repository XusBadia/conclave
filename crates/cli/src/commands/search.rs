//! `conclave-cli search` — query the knowledge base.

use anyhow::{anyhow, Result};
use clap::Args;

use conclave_rag::{search, KnowledgeStore, SearchMode, SearchRequest};

use super::CommandContext;
use crate::embedder;

/// Arguments for the `search` subcommand.
#[derive(Debug, Args)]
pub(crate) struct SearchArgs {
    /// Query text.
    #[arg(value_name = "QUERY")]
    pub(crate) query: String,

    /// Workspace to query (defaults to `config.general.default_workspace`).
    #[arg(long, value_name = "NAME")]
    pub(crate) workspace: Option<String>,

    /// Number of hits to return.
    #[arg(long, default_value_t = 8, value_name = "N")]
    pub(crate) top_k: usize,

    /// Retrieval mode: `hybrid` (default), `bm25` or `dense`.
    #[arg(long, default_value = "hybrid", value_name = "MODE")]
    pub(crate) mode: String,
}

/// Execute the `search` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: SearchArgs) -> Result<()> {
    let mode = SearchMode::parse(&args.mode).ok_or_else(|| {
        anyhow!(
            "invalid mode `{m}` — expected hybrid|bm25|dense",
            m = args.mode
        )
    })?;
    let db = ctx.workspace_db(args.workspace.as_deref());
    if !db.exists() {
        return Err(anyhow!(
            "no knowledge base at {} — run `conclave-cli ingest` first",
            db.display()
        ));
    }

    let store = KnowledgeStore::open(&db, ctx.config.knowledge.embedding_dim)?;
    let embedder = embedder::resolve(&ctx.paths, &ctx.config.knowledge)?;

    let hits = search(
        &store,
        embedder.as_ref(),
        &ctx.config.knowledge,
        &SearchRequest {
            query: args.query.clone(),
            top_k: args.top_k,
            mode,
        },
    )?;

    if hits.is_empty() {
        println!("no hits");
        return Ok(());
    }
    println!(
        "{n} hit{plural} (mode={mode:?}):",
        n = hits.len(),
        plural = if hits.len() == 1 { "" } else { "s" }
    );
    for (rank, hit) in hits.iter().enumerate() {
        let title = hit.chunk.title.as_deref().unwrap_or("(untitled)");
        println!(
            "\n[{rank:>2}] score={score:.4}  {format}  {title}",
            rank = rank + 1,
            score = hit.score,
            format = hit.chunk.format.label(),
            title = title,
        );
        println!("     {path}", path = hit.chunk.path.display());
        let snippet = preview(&hit.chunk.text, 280);
        for line in snippet.lines() {
            println!("     │ {line}");
        }
    }
    Ok(())
}

fn preview(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}
