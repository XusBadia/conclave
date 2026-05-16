//! Retrieval-Augmented Generation pipeline for Conclave.
//!
//! Phase 1 introduces the full ingestion path. Public surface so far:
//!
//! - [`extract`]: text extraction for PDF / DOCX / TXT / MD / HTML.
//! - [`chunk`]: sentence-aware token-based chunking.
//!
//! Embeddings, storage, and the orchestration pipeline land in subsequent
//! commits.

pub mod chunk;
pub mod extract;

pub use chunk::{chunk_text, Chunk, ChunkParams};
pub use extract::{extract_from_path, DocType, ExtractedText};
