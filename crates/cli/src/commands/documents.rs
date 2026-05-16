//! `conclave-cli documents` — inspect ingested documents.

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use super::CommandContext;

/// Arguments for the `documents` subcommand.
#[derive(Debug, Args)]
pub(crate) struct DocumentsArgs {
    #[command(subcommand)]
    action: DocumentsAction,
}

#[derive(Debug, Subcommand)]
enum DocumentsAction {
    /// List every document in the active workspace.
    List,
    /// Show metadata + a sample of the first chunk.
    Show {
        /// Document id (as listed by `documents list`).
        id: String,
    },
    /// Remove a document and its chunks + vectors from the workspace.
    Remove {
        /// Document id.
        id: String,
        /// Required confirmation: deletion is unrecoverable.
        #[arg(long)]
        confirm: bool,
    },
}

pub(crate) async fn run(ctx: &CommandContext, args: DocumentsArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let embedder = ctx.default_embedder();
    let repo = ctx.open_repository(&workspace, &embedder).await?;

    match args.action {
        DocumentsAction::List => {
            let listed = repo.list()?;
            if listed.is_empty() {
                println!("(no documents in workspace `{}`)", workspace.id);
            } else {
                for d in listed {
                    println!(
                        "{:<32} {:<6} {:<10} {}",
                        d.id,
                        format!("{:?}", d.doc_type).to_lowercase(),
                        format!("{:?}", d.status).to_lowercase(),
                        d.title,
                    );
                }
            }
        }
        DocumentsAction::Show { id } => {
            let details = repo
                .show(&id)?
                .ok_or_else(|| anyhow!("document `{id}` not found in workspace `{}`", workspace.id))?;
            let record = &details.record;
            println!("id:           {}", record.id);
            println!("title:        {}", record.title);
            println!("type:         {:?}", record.doc_type);
            println!("status:       {:?}", record.status);
            println!("sha256:       {}", record.sha256);
            println!("ingested_at:  {}", record.ingested_at.to_rfc3339());
            println!("source_path:  {}", record.source_path.display());
            println!("copied_path:  {}", record.copied_path.display());
            println!("chunks:       {}", details.chunk_count);
            if let Some(sample) = details.sample_text {
                let preview: String = sample.chars().take(500).collect();
                println!("\n--- first chunk (up to 500 chars) ---\n{preview}");
            }
        }
        DocumentsAction::Remove { id, confirm } => {
            if !confirm {
                anyhow::bail!(
                    "refusing to remove document `{id}` without --confirm (this is unrecoverable)"
                );
            }
            let removed = repo.remove(&id).await?;
            if removed {
                println!("removed document `{id}` from workspace `{}`", workspace.id);
            } else {
                println!("document `{id}` not found in workspace `{}`", workspace.id);
            }
        }
    }
    Ok(())
}
