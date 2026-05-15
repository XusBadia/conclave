//! Retrieval-Augmented Generation pipeline for Conclave.
//!
//! Phase 0 only delivers the basic chunking primitive and the data types that
//! later phases (ingestion, embeddings, hybrid search) will fill in.

use serde::{Deserialize, Serialize};

/// A chunk of source text plus the byte range it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// 0-based ordinal position inside the source document.
    pub index: usize,
    /// Byte offset where this chunk starts inside the source string.
    pub start: usize,
    /// Byte offset where this chunk ends (exclusive) inside the source string.
    pub end: usize,
    /// The chunk contents.
    pub text: String,
}

/// Parameters controlling chunking behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkParams {
    /// Approximate chunk size in characters.
    pub chunk_size: usize,
    /// Overlap, in characters, between adjacent chunks.
    pub overlap: usize,
}

impl ChunkParams {
    /// Construct a new set of chunking parameters.
    ///
    /// # Errors
    /// Returns [`conclave_core::Error::Rag`] if `chunk_size` is zero or
    /// `overlap >= chunk_size`.
    pub fn new(chunk_size: usize, overlap: usize) -> conclave_core::Result<Self> {
        if chunk_size == 0 {
            return Err(conclave_core::Error::Rag("chunk_size must be > 0".into()));
        }
        if overlap >= chunk_size {
            return Err(conclave_core::Error::Rag(
                "overlap must be < chunk_size".into(),
            ));
        }
        Ok(Self {
            chunk_size,
            overlap,
        })
    }
}

/// Split a piece of text into overlapping character-aware chunks.
///
/// Chunk boundaries are aligned to Unicode codepoint boundaries to avoid
/// slicing through multibyte characters.
pub fn chunk_text(text: &str, params: ChunkParams) -> Vec<Chunk> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let total_chars = text.chars().count();
    let step = params.chunk_size.saturating_sub(params.overlap).max(1);

    let mut chunk_index = 0usize;
    let mut start_char = 0usize;
    while start_char < total_chars {
        let end_char = (start_char + params.chunk_size).min(total_chars);
        let start_byte = byte_index_for_char(text, start_char);
        let end_byte = byte_index_for_char(text, end_char);
        out.push(Chunk {
            index: chunk_index,
            start: start_byte,
            end: end_byte,
            text: text[start_byte..end_byte].to_owned(),
        });
        if end_char == total_chars {
            break;
        }
        chunk_index += 1;
        start_char += step;
    }
    out
}

fn byte_index_for_char(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(b, _)| b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_chunks() {
        let params = ChunkParams::new(10, 2).unwrap();
        assert!(chunk_text("", params).is_empty());
    }

    #[test]
    fn single_short_chunk() {
        let params = ChunkParams::new(100, 10).unwrap();
        let out = chunk_text("hello", params);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "hello");
        assert_eq!(out[0].start, 0);
        assert_eq!(out[0].end, 5);
    }

    #[test]
    fn overlapping_chunks_step_correctly() {
        // chunk_size=4, overlap=1 ⇒ step=3
        // "abcdefghij" → [0,4) [3,7) [6,10)
        let params = ChunkParams::new(4, 1).unwrap();
        let out = chunk_text("abcdefghij", params);
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["abcd", "defg", "ghij"]);
        for (i, chunk) in out.iter().enumerate() {
            assert_eq!(chunk.index, i);
        }
        // Final chunk reaches the end of the input.
        assert_eq!(out.last().unwrap().end, 10);
    }

    #[test]
    fn last_short_chunk_when_no_clean_step() {
        // chunk_size=4, overlap=1 ⇒ step=3, on 9-char input:
        // [0,4) [3,7) [6,9)
        let params = ChunkParams::new(4, 1).unwrap();
        let out = chunk_text("abcdefghi", params);
        let texts: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["abcd", "defg", "ghi"]);
    }

    #[test]
    fn chunks_respect_unicode_boundaries() {
        let text = "áéíóú-ñçü";
        let params = ChunkParams::new(3, 0).unwrap();
        let out = chunk_text(text, params);
        let joined: String = out.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(joined, text);
        for chunk in &out {
            assert!(text.is_char_boundary(chunk.start));
            assert!(text.is_char_boundary(chunk.end));
        }
    }

    #[test]
    fn invalid_params_rejected() {
        assert!(ChunkParams::new(0, 0).is_err());
        assert!(ChunkParams::new(10, 10).is_err());
        assert!(ChunkParams::new(10, 11).is_err());
    }
}
