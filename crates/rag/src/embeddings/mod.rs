//! Embedding back-ends.
//!
//! The [`Embedder`] trait is the only thing the rest of the crate depends on,
//! so callers can swap [`MockEmbedder`] (deterministic, used in tests) for
//! [`FastEmbedEmbedder`] (real ONNX model downloaded at first use) without
//! touching ingestion or search code.

use conclave_core::{Error, Result};

/// A text embedder.
///
/// Implementations are expected to be deterministic for identical input:
/// the same `(model, text)` pair must always produce the same vector.
/// Vectors are returned L2-normalised so that cosine similarity collapses
/// to a dot product.
pub trait Embedder: std::fmt::Debug + Send + Sync {
    /// Stable identifier (e.g. `"mock"`, `"bge-small-en-v1.5"`).
    fn model_id(&self) -> &str;

    /// Output dimension of the produced vectors.
    fn dimension(&self) -> usize;

    /// Embed a batch of texts. Order in the output matches order in the input.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Embed a single text. Default impl wraps [`Embedder::embed`].
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self.embed(&[text])?;
        out.pop().ok_or_else(|| {
            Error::Rag("embedder returned no vector for single-text input".to_owned())
        })
    }
}

/// Deterministic, dependency-free embedder used in tests.
///
/// Vectors are computed from a Blake3 hash of the input, expanded into the
/// requested dimension and L2-normalised. Texts that share a long substring
/// produce vectors that are close in cosine distance, which is enough to
/// exercise the store / search code paths without dragging in a 600 MB ONNX
/// model.
#[derive(Debug, Clone)]
pub struct MockEmbedder {
    model_id: String,
    dim: usize,
}

impl MockEmbedder {
    /// Construct a mock embedder with the given output dimension.
    pub fn new(dim: usize) -> Self {
        Self {
            model_id: "mock".to_owned(),
            dim,
        }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self::new(64)
    }
}

impl Embedder for MockEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(mock_vector(t, self.dim));
        }
        Ok(out)
    }
}

/// Generate a deterministic, normalised pseudo-vector from `text`.
///
/// The implementation is intentionally simple: take a token-level bag of
/// hashes so that two texts sharing the same vocabulary end up close in
/// cosine space, then L2-normalise.
fn mock_vector(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim.max(1)];
    for token in tokenize_for_mock(text) {
        let hash = blake3::hash(token.as_bytes());
        let bytes = hash.as_bytes();
        // Distribute token contribution across all dimensions: index by the
        // first 8 bytes, sign by the parity of the byte.
        for (i, b) in bytes.iter().enumerate().take(dim) {
            let idx = (usize::from(*b) + i) % dim.max(1);
            let sign = if b % 2 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign * (f32::from(*b) / 255.0);
        }
    }
    l2_normalize(&mut v);
    v
}

fn tokenize_for_mock(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

/// L2-normalise a vector in place. Zero vectors are left alone.
pub fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity between two pre-normalised vectors.
///
/// Returns the dot product. Callers are expected to pass vectors that are
/// already unit-length (which every [`Embedder`] in this crate produces).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(feature = "fastembed-backend")]
mod fastembed_backend;
#[cfg(feature = "fastembed-backend")]
pub use fastembed_backend::FastEmbedEmbedder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_is_deterministic() {
        let e = MockEmbedder::new(32);
        let a = e.embed_one("infarto agudo de miocardio").unwrap();
        let b = e.embed_one("infarto agudo de miocardio").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn mock_vectors_are_unit_length() {
        let e = MockEmbedder::new(48);
        let v = e.embed_one("hello world hello world").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
    }

    #[test]
    fn mock_close_for_similar_text_far_for_different() {
        let e = MockEmbedder::new(128);
        let cardio_a = e
            .embed_one("infarto agudo de miocardio reperfusión")
            .unwrap();
        let cardio_b = e
            .embed_one("infarto miocardio reperfusión primaria")
            .unwrap();
        let neuro = e
            .embed_one("ictus isquémico trombolisis intravenosa")
            .unwrap();

        let close = cosine_similarity(&cardio_a, &cardio_b);
        let far = cosine_similarity(&cardio_a, &neuro);
        assert!(close > far, "close={close} far={far}");
    }

    #[test]
    fn empty_text_produces_finite_vector() {
        let e = MockEmbedder::new(16);
        let v = e.embed_one("").unwrap();
        assert_eq!(v.len(), 16);
        assert!(v.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn batch_matches_one_by_one() {
        let e = MockEmbedder::new(24);
        let inputs = ["alpha", "beta gamma", "delta"];
        let batch = e.embed(&inputs).unwrap();
        for (i, text) in inputs.iter().enumerate() {
            assert_eq!(batch[i], e.embed_one(text).unwrap());
        }
    }
}
