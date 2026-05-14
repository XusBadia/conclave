//! `fastembed`-backed embedder.
//!
//! Lives behind the `fastembed-backend` feature so air-gapped builds can
//! drop the ONNX Runtime download. Production binaries (CLI, future Tauri
//! shell) enable it; tests rely on [`MockEmbedder`](super::MockEmbedder).

use std::path::{Path, PathBuf};

use conclave_core::{Error, Result};

use super::{l2_normalize, Embedder};

/// `fastembed`-backed embedder. Downloads the ONNX model the first time it
/// is constructed and caches it under `cache_dir/models/<model-id>/`.
#[derive(Debug)]
pub struct FastEmbedEmbedder {
    model_id: String,
    dim: usize,
    inner: fastembed::TextEmbedding,
}

impl FastEmbedEmbedder {
    /// Build a new embedder backed by a [`fastembed::EmbeddingModel`].
    ///
    /// `cache_dir` should point at the application cache directory; the model
    /// files end up under `<cache_dir>/models/`.
    pub fn new(model_id: &str, expected_dim: usize, cache_dir: impl AsRef<Path>) -> Result<Self> {
        let model = resolve_model(model_id)?;
        let cache: PathBuf = cache_dir.as_ref().join("models");
        std::fs::create_dir_all(&cache).map_err(|e| Error::io_at(&cache, e))?;

        let opts = fastembed::InitOptions::new(model).with_cache_dir(cache);
        let inner = fastembed::TextEmbedding::try_new(opts)
            .map_err(|e| Error::Rag(format!("fastembed init failed: {e}")))?;

        Ok(Self {
            model_id: model_id.to_owned(),
            dim: expected_dim,
            inner,
        })
    }
}

fn resolve_model(id: &str) -> Result<fastembed::EmbeddingModel> {
    let model = match id {
        "bge-small-en-v1.5" => fastembed::EmbeddingModel::BGESmallENV15,
        "bge-base-en-v1.5" => fastembed::EmbeddingModel::BGEBaseENV15,
        "bge-large-en-v1.5" => fastembed::EmbeddingModel::BGELargeENV15,
        "bge-m3" => fastembed::EmbeddingModel::BGELargeENV15Q,
        other => {
            return Err(Error::Rag(format!(
                "unknown embedding model `{other}`; supported: bge-small-en-v1.5, \
                 bge-base-en-v1.5, bge-large-en-v1.5, bge-m3"
            )));
        }
    };
    Ok(model)
}

impl Embedder for FastEmbedEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let owned: Vec<String> = texts.iter().map(|s| (*s).to_owned()).collect();
        let mut vectors = self
            .inner
            .embed(owned, None)
            .map_err(|e| Error::Rag(format!("fastembed embed failed: {e}")))?;
        for v in &mut vectors {
            if v.len() != self.dim {
                return Err(Error::Rag(format!(
                    "embedder produced {actual}-dim vector but config expects {expected}",
                    actual = v.len(),
                    expected = self.dim
                )));
            }
            l2_normalize(v);
        }
        Ok(vectors)
    }
}
