//! End-to-end ingestion pipeline.
//!
//! Recursively walks a path, identifies supported documents, then loads,
//! chunks, embeds and persists each one. Deduplication is by Blake3 content
//! hash, so re-running ingestion is idempotent: a document whose normalised
//! text has not changed gets skipped.

use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use conclave_core::{Error, Result};

use crate::chunking::{chunk_text, ChunkParams};
use crate::embeddings::Embedder;
use crate::loaders::{load_path, DocumentFormat};
use crate::store::{DocumentInsert, KnowledgeStore};

/// Inputs to [`ingest_path`].
#[derive(Debug)]
pub struct IngestRequest<'a> {
    /// Root path to walk. May be a single file or a directory.
    pub root: &'a Path,
    /// Chunking parameters.
    pub chunk: ChunkParams,
    /// When true, walk and report but never write to the store.
    pub dry_run: bool,
}

/// Per-document outcome of an ingestion pass.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DocumentOutcome {
    /// New document was inserted.
    Inserted {
        /// Source path.
        path: PathBuf,
        /// Detected format.
        format: DocumentFormat,
        /// Number of chunks stored.
        chunks: usize,
    },
    /// Document already present with identical content; skipped.
    Unchanged {
        /// Source path.
        path: PathBuf,
    },
    /// Same path, but the content has changed: previous version replaced.
    Replaced {
        /// Source path.
        path: PathBuf,
        /// Detected format.
        format: DocumentFormat,
        /// Number of chunks stored.
        chunks: usize,
    },
    /// File was visited but skipped (unsupported extension).
    Skipped {
        /// Source path.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// File could not be loaded. Stored verbatim so dry-runs surface
    /// parsing problems without aborting the whole batch.
    Failed {
        /// Source path.
        path: PathBuf,
        /// Display of the underlying error.
        error: String,
    },
}

impl DocumentOutcome {
    /// Path the outcome refers to.
    pub fn path(&self) -> &Path {
        match self {
            Self::Inserted { path, .. }
            | Self::Replaced { path, .. }
            | Self::Unchanged { path }
            | Self::Skipped { path, .. }
            | Self::Failed { path, .. } => path,
        }
    }
}

/// Summary of an [`ingest_path`] run.
#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    /// Total number of files visited (regardless of outcome).
    pub visited: usize,
    /// Per-document outcomes in walk order.
    pub outcomes: Vec<DocumentOutcome>,
}

impl IngestReport {
    /// Counts inserted documents.
    pub fn inserted(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, DocumentOutcome::Inserted { .. }))
            .count()
    }
    /// Counts replaced documents.
    pub fn replaced(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, DocumentOutcome::Replaced { .. }))
            .count()
    }
    /// Counts unchanged documents.
    pub fn unchanged(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, DocumentOutcome::Unchanged { .. }))
            .count()
    }
    /// Counts skipped documents.
    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, DocumentOutcome::Skipped { .. }))
            .count()
    }
    /// Counts failed documents.
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, DocumentOutcome::Failed { .. }))
            .count()
    }
}

/// Walk `root`, ingest every supported document, return a [`IngestReport`].
pub fn ingest_path(
    store: &mut KnowledgeStore,
    embedder: &dyn Embedder,
    req: &IngestRequest<'_>,
) -> Result<IngestReport> {
    let root = req.root;
    if !root.exists() {
        return Err(Error::Rag(format!(
            "ingestion root does not exist: {}",
            root.display()
        )));
    }
    let mut report = IngestReport::default();

    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_map(std::result::Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        report.visited += 1;

        let Some(format) = DocumentFormat::from_path(path) else {
            report.outcomes.push(DocumentOutcome::Skipped {
                path: path.to_path_buf(),
                reason: "unsupported extension".to_owned(),
            });
            continue;
        };

        match handle_one(store, embedder, req, path, format) {
            Ok(outcome) => report.outcomes.push(outcome),
            Err(e) => report.outcomes.push(DocumentOutcome::Failed {
                path: path.to_path_buf(),
                error: e.to_string(),
            }),
        }
    }
    Ok(report)
}

fn handle_one(
    store: &mut KnowledgeStore,
    embedder: &dyn Embedder,
    req: &IngestRequest<'_>,
    path: &Path,
    _format: DocumentFormat,
) -> Result<DocumentOutcome> {
    let doc = load_path(path)?;
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let hash = *blake3::hash(doc.text.as_bytes()).as_bytes();

    if let Some(existing) = store.find_document_by_path(&abs)? {
        if existing.content_hash == hash {
            return Ok(DocumentOutcome::Unchanged { path: abs });
        }
    }
    if req.dry_run {
        return Ok(DocumentOutcome::Inserted {
            path: abs,
            format: doc.format,
            chunks: chunk_text(&doc.text, req.chunk).len(),
        });
    }

    let was_existing = store.find_document_by_path(&abs)?.is_some();
    let chunks = chunk_text(&doc.text, req.chunk);
    if chunks.is_empty() {
        return Ok(DocumentOutcome::Skipped {
            path: abs,
            reason: "document produced no chunks (empty text)".to_owned(),
        });
    }
    let chunk_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let embeddings = embedder.embed(&chunk_texts)?;
    let insert = DocumentInsert {
        path: &abs,
        format: doc.format,
        title: doc.title.as_deref(),
        text_bytes: doc.text.len(),
        content_hash: hash,
        chunks: &chunks,
        embeddings: &embeddings,
    };
    store.upsert_document(&insert)?;

    let chunk_count = chunks.len();
    Ok(if was_existing {
        DocumentOutcome::Replaced {
            path: abs,
            format: doc.format,
            chunks: chunk_count,
        }
    } else {
        DocumentOutcome::Inserted {
            path: abs,
            format: doc.format,
            chunks: chunk_count,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::MockEmbedder;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn walks_directory_and_skips_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "guideline.md", "# Cardio\n\nReperfusión.\n");
        write(tmp.path(), "ignore.bin", "binary blob");
        write(tmp.path(), "notes.txt", "Plain text note.");

        let embedder = MockEmbedder::new(64);
        let mut store = KnowledgeStore::open_in_memory(64).unwrap();
        let report = ingest_path(
            &mut store,
            &embedder,
            &IngestRequest {
                root: tmp.path(),
                chunk: ChunkParams::new(120, 16).unwrap(),
                dry_run: false,
            },
        )
        .unwrap();

        assert_eq!(report.visited, 3);
        assert_eq!(report.inserted(), 2);
        assert_eq!(report.skipped(), 1);
        let stats = store.stats().unwrap();
        assert_eq!(stats.documents, 2);
    }

    #[test]
    fn reingest_is_idempotent_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "g.md", "# Sepsis\n\nLactato sérico.\n");
        let embedder = MockEmbedder::new(64);
        let mut store = KnowledgeStore::open_in_memory(64).unwrap();
        let req = IngestRequest {
            root: tmp.path(),
            chunk: ChunkParams::new(120, 16).unwrap(),
            dry_run: false,
        };
        let first = ingest_path(&mut store, &embedder, &req).unwrap();
        assert_eq!(first.inserted(), 1);
        let second = ingest_path(&mut store, &embedder, &req).unwrap();
        assert_eq!(second.inserted(), 0);
        assert_eq!(second.unchanged(), 1);
    }

    #[test]
    fn reingest_replaces_when_content_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write(tmp.path(), "g.md", "# Sepsis\n\nLactato sérico.\n");
        let embedder = MockEmbedder::new(64);
        let mut store = KnowledgeStore::open_in_memory(64).unwrap();
        let req = IngestRequest {
            root: tmp.path(),
            chunk: ChunkParams::new(120, 16).unwrap(),
            dry_run: false,
        };
        ingest_path(&mut store, &embedder, &req).unwrap();
        std::fs::write(&p, "# Sepsis\n\nLactato y procalcitonina.\n").unwrap();
        let second = ingest_path(&mut store, &embedder, &req).unwrap();
        assert_eq!(second.replaced(), 1);
        assert_eq!(second.inserted(), 0);
    }

    #[test]
    fn dry_run_does_not_mutate_store() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "g.md", "# Cardio\n\nReperfusión.\n");
        let embedder = MockEmbedder::new(64);
        let mut store = KnowledgeStore::open_in_memory(64).unwrap();
        let report = ingest_path(
            &mut store,
            &embedder,
            &IngestRequest {
                root: tmp.path(),
                chunk: ChunkParams::new(120, 16).unwrap(),
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(report.inserted(), 1);
        assert_eq!(store.stats().unwrap().documents, 0);
    }
}
