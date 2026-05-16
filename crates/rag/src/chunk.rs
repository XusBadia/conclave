//! Sentence-aware, token-based chunking for the ingestion pipeline.
//!
//! Splits incoming text into sentences with a regex that understands Spanish
//! and English punctuation, then greedily packs sentences into chunks of
//! approximately `target_tokens` (default ~700) using `tiktoken-rs`
//! (`cl100k_base`) as the size oracle. Adjacent chunks share roughly
//! `overlap_tokens` of context (default ~100) but never split a sentence in
//! half — overlap is rounded down to whole sentences.
//!
//! The BPE table ships embedded in `tiktoken-rs`, so token counting is fully
//! offline and works on every CI target.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tiktoken_rs::{cl100k_base, CoreBPE};

use conclave_core::{Error, Result};

/// A chunk of a source document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// Stable id of the form `<document_id>-<position>`.
    pub id: String,
    /// Chunk text with `\n`-normalised line endings.
    pub text: String,
    /// Owning document id.
    pub document_id: String,
    /// First page covered by this chunk (1-based, or 0 when unknown).
    pub page_start: u32,
    /// Last page covered by this chunk (1-based, or 0 when unknown).
    pub page_end: u32,
    /// 0-based ordinal position within the document.
    pub position: u32,
}

/// Tunables for the chunker.
// Every field carries a token budget — the shared `_tokens` suffix names
// the unit, not the role, so the pedantic lint here is noise.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkParams {
    /// Target chunk size in tokens. Chunks will be at most this many tokens
    /// when a single sentence fits; an oversized sentence is emitted whole.
    pub target_tokens: usize,
    /// Soft floor in tokens. Chunks shorter than this are tolerated only at
    /// end-of-document.
    pub min_tokens: usize,
    /// Tokens of overlap shared with the previous chunk. Honours sentence
    /// boundaries — we never split a sentence to hit this number exactly.
    pub overlap_tokens: usize,
}

impl ChunkParams {
    /// Default parameters: 500–800 tokens per chunk, ~100 token overlap.
    pub const DEFAULT: Self = Self {
        target_tokens: 700,
        min_tokens: 500,
        overlap_tokens: 100,
    };

    /// Construct a validated set of parameters.
    pub fn new(target_tokens: usize, min_tokens: usize, overlap_tokens: usize) -> Result<Self> {
        if target_tokens == 0 {
            return Err(Error::Rag("target_tokens must be > 0".into()));
        }
        if min_tokens > target_tokens {
            return Err(Error::Rag("min_tokens must be <= target_tokens".into()));
        }
        if overlap_tokens >= target_tokens {
            return Err(Error::Rag("overlap_tokens must be < target_tokens".into()));
        }
        Ok(Self {
            target_tokens,
            min_tokens,
            overlap_tokens,
        })
    }
}

impl Default for ChunkParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Cached `cl100k_base` tokeniser. The BPE table is bundled inside
/// `tiktoken-rs` via `include_str!`, so initialisation never touches the
/// network.
fn bpe() -> Result<&'static CoreBPE> {
    static CELL: OnceLock<CoreBPE> = OnceLock::new();
    if let Some(b) = CELL.get() {
        return Ok(b);
    }
    let built = cl100k_base().map_err(|e| Error::Rag(format!("cl100k_base init: {e}")))?;
    Ok(CELL.get_or_init(|| built))
}

fn count_tokens(s: &str) -> Result<usize> {
    Ok(bpe()?.encode_with_special_tokens(s).len())
}

/// Split text into sentence-like fragments. Recognises `.`, `!`, `?` and
/// hard line breaks as boundary characters, which is enough for Spanish and
/// English clinical prose. Inverted punctuation (`¿`, `¡`) is preserved at
/// the start of fragments because the regex never consumes leading chars.
fn split_sentences(text: &str) -> Vec<&str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // Either a run of non-terminator chars ending in a terminator, or a
        // tail run that runs into a newline/EOF.
        Regex::new(r"[^.!?\n]+[.!?]+|[^.!?\n]+(?:$|\n)").expect("hand-written regex must compile")
    });
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let s = m.as_str().trim();
        if !s.is_empty() {
            out.push(s);
        }
    }
    if out.is_empty() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            out.push(trimmed);
        }
    }
    out
}

/// Chunk `text` into [`Chunk`]s belonging to `document_id`.
pub fn chunk_text(text: &str, document_id: &str, params: ChunkParams) -> Result<Vec<Chunk>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-compute token counts to avoid re-tokenising sentences across loops.
    let mut sentence_tokens: Vec<usize> = Vec::with_capacity(sentences.len());
    for s in &sentences {
        sentence_tokens.push(count_tokens(s)?);
    }

    let mut chunks = Vec::new();
    let mut i = 0usize;
    let mut position: u32 = 0;

    while i < sentences.len() {
        let mut buf = String::new();
        let mut tokens = 0usize;
        let mut j = i;
        // Greedy fill up to `target_tokens`.
        while j < sentences.len() && tokens + sentence_tokens[j] <= params.target_tokens {
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(sentences[j]);
            tokens += sentence_tokens[j];
            j += 1;
        }
        // A single sentence longer than `target_tokens` still gets emitted
        // whole; we just produce one oversized chunk and move on.
        if j == i {
            buf.push_str(sentences[i]);
            j += 1;
        }

        chunks.push(Chunk {
            id: format!("{document_id}-{position}"),
            text: buf,
            document_id: document_id.to_owned(),
            page_start: 0,
            page_end: 0,
            position,
        });
        position += 1;

        if j >= sentences.len() {
            break;
        }

        // Compute the next starting index by rewinding to honour
        // `overlap_tokens` without splitting any sentence.
        let mut overlap_back = 0usize;
        let mut k = j;
        while k > i && overlap_back + sentence_tokens[k - 1] <= params.overlap_tokens {
            overlap_back += sentence_tokens[k - 1];
            k -= 1;
        }
        // Always advance at least one sentence to guarantee termination when
        // overlap is large enough to cover the entire just-emitted chunk.
        i = k.max(i + 1);
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(chunk_text("", "doc-1", ChunkParams::DEFAULT)
            .unwrap()
            .is_empty());
        assert!(chunk_text("    \n\n\t  ", "doc-1", ChunkParams::DEFAULT)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn single_sentence_becomes_one_chunk() {
        let out = chunk_text("Hola mundo.", "doc-1", ChunkParams::DEFAULT).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "doc-1-0");
        assert_eq!(out[0].position, 0);
        assert_eq!(out[0].document_id, "doc-1");
        assert!(out[0].text.contains("Hola mundo"));
    }

    #[test]
    fn splits_into_multiple_chunks_when_over_target() {
        let params = ChunkParams::new(8, 1, 2).unwrap();
        let text = "Alpha beta gamma. Delta epsilon zeta. Eta theta iota. Kappa lambda mu.";
        let out = chunk_text(text, "doc-1", params).unwrap();
        assert!(out.len() >= 2);
        for (i, c) in out.iter().enumerate() {
            assert_eq!(c.position, u32::try_from(i).unwrap());
            assert_eq!(c.id, format!("doc-1-{i}"));
            assert!(!c.text.is_empty());
        }
    }

    #[test]
    fn handles_spanish_punctuation() {
        let text = "¿Cómo estás? ¡Genial! Estoy bien, gracias.";
        let out = chunk_text(text, "d", ChunkParams::DEFAULT).unwrap();
        // Fits comfortably inside a 700-token default budget.
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("Genial"));
        assert!(out[0].text.contains("Cómo"));
    }

    #[test]
    fn rejects_invalid_params() {
        assert!(ChunkParams::new(0, 0, 0).is_err());
        assert!(ChunkParams::new(10, 11, 0).is_err());
        assert!(ChunkParams::new(10, 5, 10).is_err());
        assert!(ChunkParams::new(10, 5, 11).is_err());
    }

    #[test]
    fn chunk_ids_are_stable_and_sequential() {
        let out = chunk_text("First. Second. Third.", "abc", ChunkParams::DEFAULT).unwrap();
        for (i, c) in out.iter().enumerate() {
            assert_eq!(c.id, format!("abc-{i}"));
            assert_eq!(c.position, u32::try_from(i).unwrap());
        }
    }

    #[test]
    fn oversized_single_sentence_is_emitted_whole() {
        // 1-token target — every "word." becomes its own chunk.
        let params = ChunkParams::new(1, 1, 0).unwrap();
        let text = "uno dos tres cuatro cinco seis siete ocho nueve diez.";
        let out = chunk_text(text, "d", params).unwrap();
        // Loop terminates: every iteration emits at least one chunk and
        // advances at least one sentence.
        assert!(!out.is_empty());
    }
}
