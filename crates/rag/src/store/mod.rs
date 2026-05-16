//! Per-workspace storage: metadata (`SQLite`), vectors (`LanceDB`) and file
//! copies.
//!
//! The three components live behind one façade — [`DocumentRepository`] —
//! which the ingestion pipeline talks to. Splitting them as separate modules
//! keeps each backend's concerns isolated and makes them straightforward to
//! unit-test.

mod metadata;
mod repository;
mod vector;

pub use metadata::{DocumentRecord, DocumentStatus, MetadataStore};
pub use repository::{DocumentRepository, RepositoryLayout};
pub use vector::{VectorHit, VectorStore};
