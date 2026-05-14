//! Embedder selection for the CLI.
//!
//! Resolution rules:
//! 1. `embedding_model = "mock"` always selects the deterministic
//!    [`MockEmbedder`]. Useful for air-gapped smoke tests and for CI.
//! 2. Anything else requires the `fastembed-backend` feature; if it is
//!    enabled we hand back a [`FastEmbedEmbedder`], otherwise we surface a
//!    clear error so the user knows to rebuild with the feature on.

use std::sync::Arc;

use anyhow::{bail, Result};

use conclave_core::{paths::Paths, KnowledgeConfig};
use conclave_rag::{Embedder, MockEmbedder};

/// Resolve the configured embedder.
pub(crate) fn resolve(paths: &Paths, knowledge: &KnowledgeConfig) -> Result<Arc<dyn Embedder>> {
    let id = knowledge.embedding_model.as_str();
    if id.eq_ignore_ascii_case("mock") {
        tracing::debug!(dim = knowledge.embedding_dim, "using mock embedder");
        return Ok(Arc::new(MockEmbedder::new(knowledge.embedding_dim)));
    }

    #[cfg(feature = "fastembed-backend")]
    {
        let embedder = conclave_rag::embeddings::FastEmbedEmbedder::new(
            id,
            knowledge.embedding_dim,
            paths.cache_dir(),
        )?;
        tracing::info!(
            model = id,
            dim = knowledge.embedding_dim,
            "fastembed embedder ready"
        );
        Ok(Arc::new(embedder))
    }

    #[cfg(not(feature = "fastembed-backend"))]
    {
        let _ = paths;
        bail!(
            "embedding_model = `{id}` requires the `fastembed-backend` feature.\n\
             Either rebuild with `--features fastembed-backend` or set \
             `knowledge.embedding_model = \"mock\"` in conclave.toml."
        );
    }
}
