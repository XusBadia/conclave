//! Hybrid search: BM25 + dense, fused with Reciprocal Rank Fusion.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use conclave_core::{KnowledgeConfig, Result};

use crate::embeddings::Embedder;
use crate::store::{KnowledgeStore, StoredChunk};

/// Retrieval mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// BM25 only — purely lexical.
    Bm25,
    /// Dense embeddings only — semantic.
    Dense,
    /// Reciprocal Rank Fusion of BM25 and dense.
    Hybrid,
}

impl SearchMode {
    /// Parse a string used by the CLI / config.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bm25" | "lex" | "lexical" => Some(Self::Bm25),
            "dense" | "vec" | "vector" | "semantic" => Some(Self::Dense),
            "hybrid" | "rrf" => Some(Self::Hybrid),
            _ => None,
        }
    }
}

/// A query against the knowledge store.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Free-form query text.
    pub query: String,
    /// Maximum number of results returned.
    pub top_k: usize,
    /// Which retrieval mode to use.
    pub mode: SearchMode,
}

/// A single search hit.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// The underlying chunk.
    pub chunk: StoredChunk,
    /// Final score (interpretation depends on the mode):
    /// - `Bm25`: BM25 score, higher is better (negated internally so we
    ///   stay consistent with the other modes).
    /// - `Dense`: cosine similarity in `[-1, 1]`.
    /// - `Hybrid`: RRF score (positive, unbounded).
    pub score: f32,
}

/// Run a search against the store.
///
/// For `Hybrid`, the request fetches `2 * top_k` candidates from each leg
/// before fusing — this gives RRF enough room to promote items that are
/// strong on one signal but missing from the other's top-K.
pub fn search(
    store: &KnowledgeStore,
    embedder: &dyn Embedder,
    cfg: &KnowledgeConfig,
    req: &SearchRequest,
) -> Result<Vec<SearchHit>> {
    if req.top_k == 0 {
        return Ok(Vec::new());
    }
    match req.mode {
        SearchMode::Bm25 => {
            let raw = store.search_bm25(&req.query, req.top_k)?;
            Ok(raw.into_iter().map(into_hit).collect())
        }
        SearchMode::Dense => {
            let qv = embedder.embed_one(&req.query)?;
            let raw = store.search_dense(&qv, req.top_k)?;
            Ok(raw.into_iter().map(into_hit).collect())
        }
        SearchMode::Hybrid => {
            let widened = req.top_k.saturating_mul(2).max(req.top_k);
            let bm25 = store.search_bm25(&req.query, widened)?;
            let qv = embedder.embed_one(&req.query)?;
            let dense = store.search_dense(&qv, widened)?;
            Ok(fuse_rrf(
                &bm25,
                &dense,
                cfg.bm25_weight,
                cfg.dense_weight,
                cfg.rrf_k,
                req.top_k,
            ))
        }
    }
}

fn rank_as_f32(rank: usize) -> f32 {
    // Ranks are bounded by `top_k` which is in the low hundreds at most, so
    // an explicit clamp avoids the precision-loss lint without changing
    // behaviour.
    f32::from(u16::try_from(rank.min(u16::MAX as usize)).unwrap_or(u16::MAX))
}

fn into_hit(c: StoredChunk) -> SearchHit {
    let score = c.score.unwrap_or(0.0);
    SearchHit { chunk: c, score }
}

/// Reciprocal Rank Fusion of two ranked lists.
///
/// `score(d) = w_bm25 / (k + rank_bm25) + w_dense / (k + rank_dense)`, with
/// missing entries contributing zero. Rank is 1-based.
fn fuse_rrf(
    bm25: &[StoredChunk],
    dense: &[StoredChunk],
    w_bm25: f32,
    w_dense: f32,
    k: f32,
    top_k: usize,
) -> Vec<SearchHit> {
    let mut scores: HashMap<i64, f32> = HashMap::new();
    let mut keep: HashMap<i64, StoredChunk> = HashMap::new();

    for (i, hit) in bm25.iter().enumerate() {
        let rank = rank_as_f32(i + 1);
        let s = w_bm25 / (k + rank);
        *scores.entry(hit.chunk_id).or_default() += s;
        keep.entry(hit.chunk_id).or_insert_with(|| hit.clone());
    }
    for (i, hit) in dense.iter().enumerate() {
        let rank = rank_as_f32(i + 1);
        let s = w_dense / (k + rank);
        *scores.entry(hit.chunk_id).or_default() += s;
        keep.entry(hit.chunk_id).or_insert_with(|| hit.clone());
    }

    let mut fused: Vec<SearchHit> = scores
        .into_iter()
        .map(|(id, score)| SearchHit {
            chunk: keep.remove(&id).expect("kept earlier"),
            score,
        })
        .collect();
    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.chunk.chunk_id.cmp(&b.chunk.chunk_id))
    });
    fused.truncate(top_k);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{chunk_text, ChunkParams};
    use crate::embeddings::MockEmbedder;
    use crate::loaders::DocumentFormat;
    use crate::store::DocumentInsert;
    use std::path::Path;

    fn make_store() -> (KnowledgeStore, MockEmbedder) {
        let embedder = MockEmbedder::new(96);
        let mut store = KnowledgeStore::open_in_memory(96).unwrap();
        let docs = [
            (
                "/fake/cardio.md",
                "Manejo del IAMCEST",
                "Reperfusión primaria con angioplastia antes de 120 minutos. \
                 Doble antiagregación con AAS y prasugrel.",
            ),
            (
                "/fake/neuro.md",
                "Ictus isquémico agudo",
                "Trombólisis intravenosa con alteplasa en ventana <4.5 h. \
                 Trombectomía mecánica para oclusión proximal.",
            ),
            (
                "/fake/sepsis.md",
                "Sepsis y shock séptico",
                "Antibiótico empírico en la primera hora. Lactato sérico, \
                 cristaloides 30 ml/kg, vasopresor noradrenalina.",
            ),
        ];
        let params = ChunkParams::new(120, 16).unwrap();
        for (path, title, text) in docs {
            let chunks = chunk_text(text, params);
            let chunk_refs: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = <MockEmbedder as Embedder>::embed(&embedder, &chunk_refs).unwrap();
            let insert = DocumentInsert {
                path: Path::new(path),
                format: DocumentFormat::Markdown,
                title: Some(title),
                text_bytes: text.len(),
                content_hash: *blake3::hash(text.as_bytes()).as_bytes(),
                chunks: &chunks,
                embeddings: &embeddings,
            };
            store.upsert_document(&insert).unwrap();
        }
        (store, embedder)
    }

    #[test]
    fn search_mode_parse() {
        assert_eq!(SearchMode::parse("bm25"), Some(SearchMode::Bm25));
        assert_eq!(SearchMode::parse("DENSE"), Some(SearchMode::Dense));
        assert_eq!(SearchMode::parse("hybrid"), Some(SearchMode::Hybrid));
        assert_eq!(SearchMode::parse("nope"), None);
    }

    #[test]
    fn hybrid_returns_top_k_at_most() {
        let (store, embedder) = make_store();
        let cfg = KnowledgeConfig::default();
        let hits = search(
            &store,
            &embedder,
            &cfg,
            &SearchRequest {
                query: "reperfusión angioplastia".to_owned(),
                top_k: 2,
                mode: SearchMode::Hybrid,
            },
        )
        .unwrap();
        assert!(hits.len() <= 2);
    }

    #[test]
    fn hybrid_top_hit_is_cardio_for_cardio_query() {
        let (store, embedder) = make_store();
        let cfg = KnowledgeConfig::default();
        let hits = search(
            &store,
            &embedder,
            &cfg,
            &SearchRequest {
                query: "reperfusión angioplastia primaria".to_owned(),
                top_k: 3,
                mode: SearchMode::Hybrid,
            },
        )
        .unwrap();
        assert_eq!(hits[0].chunk.title.as_deref(), Some("Manejo del IAMCEST"));
    }

    #[test]
    fn hybrid_top_hit_is_neuro_for_neuro_query() {
        let (store, embedder) = make_store();
        let cfg = KnowledgeConfig::default();
        let hits = search(
            &store,
            &embedder,
            &cfg,
            &SearchRequest {
                query: "alteplasa trombectomía".to_owned(),
                top_k: 3,
                mode: SearchMode::Hybrid,
            },
        )
        .unwrap();
        assert_eq!(
            hits[0].chunk.title.as_deref(),
            Some("Ictus isquémico agudo")
        );
    }

    #[test]
    fn empty_request_returns_empty() {
        let (store, embedder) = make_store();
        let cfg = KnowledgeConfig::default();
        let hits = search(
            &store,
            &embedder,
            &cfg,
            &SearchRequest {
                query: "anything".to_owned(),
                top_k: 0,
                mode: SearchMode::Hybrid,
            },
        )
        .unwrap();
        assert!(hits.is_empty());
    }
}
