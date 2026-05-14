//! Retrieval-Augmented Generation pipeline for Conclave.
//!
//! This crate owns the knowledge-base side of the application: ingestion of
//! clinical documents (Markdown, plain text, PDF, HTML, DOCX), Unicode-safe
//! [chunking](crate::chunking), pluggable [embeddings](crate::embeddings),
//! a per-workspace persistent [`KnowledgeStore`](crate::store::KnowledgeStore)
//! (`SQLite` + `sqlite-vec` for ANN + FTS5 for BM25), and a hybrid
//! [`search`](crate::search) layer that fuses both signals with Reciprocal
//! Rank Fusion.
//!
//! The crate exposes synchronous APIs intentionally: every operation here is
//! either local I/O or pure CPU work, so wrapping it in `async` would buy
//! nothing.

pub mod chunking;
pub mod embeddings;
pub mod ingest;
pub mod loaders;
pub mod search;
pub mod store;

pub use chunking::{chunk_text, Chunk, ChunkParams};
pub use embeddings::{Embedder, MockEmbedder};
pub use ingest::{ingest_path, IngestReport, IngestRequest};
pub use loaders::{load_path, Document, DocumentFormat};
pub use search::{search, SearchHit, SearchMode, SearchRequest};
pub use store::{KnowledgeStore, StoreStats, StoredChunk};
