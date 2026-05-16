//! Subcommand modules for `conclave-cli`.
//!
//! Each subcommand exposes an `Args` struct (used by `clap`) and a `run`
//! function. They share a [`CommandContext`] carrying the resolved paths,
//! parsed configuration and the optional workspace override.

use std::sync::Arc;

use anyhow::{Context, Result};

use conclave_core::{paths::Paths, Config, Workspace, WorkspaceManager};
use conclave_rag::{
    ChunkParams, DocumentRepository, Embedder, FastEmbedEmbedder, RepositoryLayout,
};

pub(crate) mod case;
pub(crate) mod deident;
pub(crate) mod documents;
pub(crate) mod evidence;
pub(crate) mod export;
pub(crate) mod feedback;
pub(crate) mod ingest;
pub(crate) mod providers;
pub(crate) mod search;
pub(crate) mod stats;
pub(crate) mod workspace;

/// Runtime context handed to each subcommand.
#[derive(Debug)]
pub(crate) struct CommandContext {
    /// Resolved application paths (config / data / cache).
    pub(crate) paths: Paths,
    /// Parsed configuration.
    pub(crate) config: Config,
    /// Workspace name passed via the global `--workspace` flag, if any.
    pub(crate) workspace_override: Option<String>,
}

impl CommandContext {
    /// Resolve the active workspace by checking, in order: per-command
    /// override (if `subcommand` set one), the global `--workspace` flag,
    /// then `config.general.default_workspace`.
    pub(crate) fn resolve_workspace(&self, subcommand_override: Option<&str>) -> Result<Workspace> {
        let name = subcommand_override
            .map(str::to_owned)
            .or_else(|| self.workspace_override.clone())
            .unwrap_or_else(|| self.config.general.default_workspace.clone());
        let manager = WorkspaceManager::new(self.paths.workspaces_dir());
        manager
            .load(&name)
            .with_context(|| format!("could not load workspace `{name}`"))
    }

    /// Open a repository for the given workspace, using the embedder
    /// supplied by the caller.
    pub(crate) async fn open_repository(
        &self,
        workspace: &Workspace,
        embedder: &Arc<dyn Embedder>,
    ) -> Result<Arc<DocumentRepository>> {
        let dir = self.paths.workspace_dir(&workspace.id);
        let layout = RepositoryLayout::new(dir);
        let repo = DocumentRepository::open(layout, embedder.dim())
            .await
            .with_context(|| {
                format!("could not open repository for workspace `{}`", workspace.id)
            })?;
        Ok(Arc::new(repo))
    }

    /// Default embedder used by ingest/search subcommands.
    // Takes `&self` even though it does not consult ctx today — future
    // versions will respect a config-driven embedder choice.
    #[allow(clippy::unused_self)]
    pub(crate) fn default_embedder(&self) -> Arc<dyn Embedder> {
        Arc::new(FastEmbedEmbedder::new())
    }

    /// Chunking parameters derived from `Config.rag`.
    pub(crate) fn chunk_params(&self) -> Result<ChunkParams> {
        ChunkParams::new(
            self.config.rag.chunk_size,
            self.config
                .rag
                .chunk_size
                .saturating_sub(self.config.rag.chunk_overlap),
            self.config.rag.chunk_overlap,
        )
        .map_err(|e| anyhow::anyhow!("invalid chunk params from config: {e}"))
    }
}
