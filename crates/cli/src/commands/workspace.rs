//! `conclave-cli workspace` — manage Conclave workspaces.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::CommandContext;

/// Arguments for the `workspace` subcommand.
#[derive(Debug, Args)]
pub(crate) struct WorkspaceArgs {
    #[command(subcommand)]
    pub(crate) action: WorkspaceAction,
}

/// Sub-actions exposed by `workspace`.
#[derive(Debug, Subcommand)]
pub(crate) enum WorkspaceAction {
    /// Print the resolved paths and active configuration summary.
    Info,
    /// Initialise the on-disk config file with defaults if it is missing.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
    /// Report knowledge-base statistics for a workspace.
    Stats {
        /// Workspace to inspect (defaults to the configured one).
        #[arg(long, value_name = "NAME")]
        workspace: Option<String>,
    },
}

/// Execute the `workspace` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: WorkspaceArgs) -> Result<()> {
    match args.action {
        WorkspaceAction::Info => {
            println!("config dir: {}", ctx.paths.config_dir().display());
            println!("data dir:   {}", ctx.paths.data_dir().display());
            println!("cache dir:  {}", ctx.paths.cache_dir().display());
            println!("config file: {}", ctx.paths.config_file().display());
            println!(
                "default workspace: {}",
                ctx.config.general.default_workspace
            );
            println!(
                "default provider: {}",
                ctx.config.providers.default.as_deref().unwrap_or("<unset>"),
            );
            println!(
                "embedding model: {}  (dim={})",
                ctx.config.knowledge.embedding_model, ctx.config.knowledge.embedding_dim,
            );
        }
        WorkspaceAction::Init { force } => {
            let path = ctx.paths.config_file();
            if path.exists() && !force {
                anyhow::bail!(
                    "config file already exists at {} — pass --force to overwrite",
                    path.display(),
                );
            }
            ctx.config.save(&path)?;
            println!("wrote default config to {}", path.display());
        }
        WorkspaceAction::Stats { workspace } => {
            let db = ctx.workspace_db(workspace.as_deref());
            if !db.exists() {
                println!(
                    "no knowledge base yet at {} — run `conclave-cli ingest` to create one",
                    db.display()
                );
                return Ok(());
            }
            let store =
                conclave_rag::KnowledgeStore::open(&db, ctx.config.knowledge.embedding_dim)?;
            let stats = store.stats()?;
            println!("knowledge base: {}", db.display());
            println!("  documents:  {}", stats.documents);
            println!("  chunks:     {}", stats.chunks);
            println!("  disk size:  {} bytes", stats.disk_bytes);
        }
    }
    Ok(())
}
