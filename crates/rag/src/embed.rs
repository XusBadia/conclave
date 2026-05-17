//! Embeddings: turn text into vectors.
//!
//! Two implementations:
//! - [`FastEmbedEmbedder`] wraps `fastembed` with the `multilingual-e5-small`
//!   model. Construction is cheap; the underlying ONNX session is initialised
//!   lazily on the first call to [`Embedder::embed`], which is also when the
//!   model weights are downloaded (~470 MB) into the user's cache directory.
//! - [`MockEmbedder`] (exposed under `#[cfg(test)]` and the `test-utils`
//!   feature) emits deterministic hash-based vectors so CI never depends on
//!   the network.
//!
//! The trait is intentionally synchronous: `fastembed` is sync internally,
//! and the pipeline wraps embedding calls in `tokio::task::spawn_blocking`
//! when used from async contexts.

use std::fmt;
use std::sync::Mutex;

use conclave_core::{Error, Result};

/// Embedding dimensionality of `multilingual-e5-small`.
pub const E5_SMALL_DIM: usize = 384;

/// Anything that can turn text into a fixed-dimension vector.
pub trait Embedder: Send + Sync + fmt::Debug {
    /// Stable identifier for telemetry and config persistence.
    fn id(&self) -> &'static str;
    /// Output dimensionality. Same for every input.
    fn dim(&self) -> usize;
    /// Embed a batch of texts. An empty input slice yields an empty output.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// `fastembed` adapter using `multilingual-e5-small`.
///
/// Construction does not touch the network or filesystem. The underlying
/// model is initialised on the first [`Embedder::embed`] call and cached for
/// the lifetime of the embedder.
pub struct FastEmbedEmbedder {
    model: Mutex<Option<fastembed::TextEmbedding>>,
    batch_size: usize,
    cache_dir: Option<std::path::PathBuf>,
}

impl FastEmbedEmbedder {
    /// Default constructor. Batch size 32 matches the Phase 1 spec. The
    /// model is loaded into the CWD-relative `.fastembed_cache` on first
    /// use — fine for the CLI and tests, where CWD is stable.
    pub const fn new() -> Self {
        Self::with_batch_size(32)
    }

    /// Construct with a custom batch size for embeddings.
    pub const fn with_batch_size(batch_size: usize) -> Self {
        Self {
            model: Mutex::new(None),
            batch_size,
            cache_dir: None,
        }
    }

    /// Pin the on-disk cache directory the underlying ONNX model is read
    /// from / downloaded to. Required for desktop launches where the CWD
    /// depends on how the binary was started (Tauri dev vs `open .app`)
    /// — without it, fastembed re-downloads on each launch flavor.
    pub fn with_cache_dir(mut self, cache_dir: impl Into<std::path::PathBuf>) -> Self {
        self.cache_dir = Some(cache_dir.into());
        self
    }
}

impl Default for FastEmbedEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for FastEmbedEmbedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let loaded = self.model.lock().is_ok_and(|m| m.is_some());
        f.debug_struct("FastEmbedEmbedder")
            .field("batch_size", &self.batch_size)
            .field("cache_dir", &self.cache_dir)
            .field("loaded", &loaded)
            .finish()
    }
}

impl Embedder for FastEmbedEmbedder {
    fn id(&self) -> &'static str {
        "fastembed:multilingual-e5-small"
    }

    fn dim(&self) -> usize {
        E5_SMALL_DIM
    }

    // The mutex guard is held across `embed` on purpose so the underlying
    // `&mut TextEmbedding` stays valid for the whole batch.
    #[allow(clippy::significant_drop_tightening)]
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self
            .model
            .lock()
            .map_err(|_| Error::Rag("fastembed lock poisoned".into()))?;
        if guard.is_none() {
            let mut opts =
                fastembed::InitOptions::new(fastembed::EmbeddingModel::MultilingualE5Small);
            if let Some(dir) = &self.cache_dir {
                std::fs::create_dir_all(dir)
                    .map_err(|e| Error::Rag(format!("fastembed cache mkdir: {e}")))?;
                opts = opts.with_cache_dir(dir.clone());
            }
            let model = fastembed::TextEmbedding::try_new(opts)
                .map_err(|e| Error::Rag(format!("fastembed init: {e}")))?;
            *guard = Some(model);
        }
        // `fastembed::TextEmbedding::embed` borrows `&mut self`, so we hold
        // the lock across the call. That serialises embed batches; fine for
        // our use — we batch up to 32 inputs per call anyway.
        let model = guard
            .as_mut()
            .ok_or_else(|| Error::Rag("fastembed model missing after init".into()))?;
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vectors = model
            .embed(refs, Some(self.batch_size))
            .map_err(|e| Error::Rag(format!("fastembed embed: {e}")))?;
        Ok(vectors)
    }
}

// ---------------------------------------------------------------------------
// Mock embedder (test + opt-in feature).
// ---------------------------------------------------------------------------

/// Deterministic mock embedder for tests and integration scenarios.
///
/// Emits 384-dimensional L2-normalised vectors derived from a per-byte FNV
/// hash of the input. Identical inputs yield identical vectors; different
/// inputs are very likely to point in different directions, which is enough
/// to drive ANN search end-to-end without touching the network.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockEmbedder;

impl MockEmbedder {
    /// Build a new mock embedder. No state is held.
    pub const fn new() -> Self {
        Self
    }
}

impl Embedder for MockEmbedder {
    fn id(&self) -> &'static str {
        "mock:hash"
    }

    fn dim(&self) -> usize {
        E5_SMALL_DIM
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| hash_vector(t)).collect())
    }
}

fn hash_vector(text: &str) -> Vec<f32> {
    let mut out = vec![0f32; E5_SMALL_DIM];
    let bytes = text.as_bytes();
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, b) in bytes.iter().enumerate() {
        state ^= u64::from(*b);
        state = state.wrapping_mul(0x100_0000_01b3);
        // On 32-bit platforms the cast truncates to the low 32 bits, which is
        // exactly what we want for slot selection — pseudo-random spread is
        // the goal, not value preservation.
        #[allow(clippy::cast_possible_truncation)]
        let mixed = state as usize;
        let slot = i.wrapping_add(mixed) % E5_SMALL_DIM;
        // Map lower 16 bits to a number in roughly [-1, 1].
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let signed = (state & 0xFFFF) as i16;
        out[slot] += f32::from(signed) / f32::from(i16::MAX);
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut out {
            *x /= norm;
        }
    } else {
        // Empty string: canonical first-axis unit vector.
        out[0] = 1.0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_id_and_dim() {
        let e = MockEmbedder::new();
        assert_eq!(e.id(), "mock:hash");
        assert_eq!(e.dim(), E5_SMALL_DIM);
    }

    #[test]
    fn mock_embed_is_deterministic() {
        let e = MockEmbedder::new();
        let a = e.embed(&["hello".into(), "world".into()]).unwrap();
        let b = e.embed(&["hello".into(), "world".into()]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].len(), E5_SMALL_DIM);
        // Vectors should be unit-length.
        let norm: f32 = a[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "expected unit vector, got norm={norm}"
        );
    }

    #[test]
    fn mock_embed_empty_input_yields_empty_output() {
        let e = MockEmbedder::new();
        assert!(e.embed(&[]).unwrap().is_empty());
    }

    #[test]
    fn mock_embed_distinguishes_inputs() {
        let e = MockEmbedder::new();
        let a = e.embed(&["lorem ipsum dolor".into()]).unwrap();
        let b = e.embed(&["completely different".into()]).unwrap();
        let dot: f32 = a[0].iter().zip(b[0].iter()).map(|(x, y)| x * y).sum();
        assert!(
            dot < 0.99,
            "two distinct inputs collapsed to ~identical vectors (dot={dot})"
        );
    }

    #[test]
    fn fastembed_embedder_constructs_without_io() {
        // No network or filesystem access should happen here.
        let e = FastEmbedEmbedder::new();
        assert_eq!(e.dim(), E5_SMALL_DIM);
        assert_eq!(e.id(), "fastembed:multilingual-e5-small");
        // Debug should report the unloaded state.
        let dbg = format!("{e:?}");
        assert!(dbg.contains("loaded: false"));
    }
}
