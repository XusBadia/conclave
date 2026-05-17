//! `DocumentRepository` — façade that the ingestion pipeline talks to.
//!
//! Combines the `SQLite` [`MetadataStore`] and the `LanceDB` [`VectorStore`]
//! plus a `documents/` directory holding immutable copies of every ingested
//! file. Atomicity is best-effort: rows go into `SQLite` under a
//! transaction, vectors go into `LanceDB` after a successful `SQLite`
//! commit, and a failure on the vector side triggers a compensating
//! `SQLite` delete with an error returned.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use sha2::{Digest, Sha256};

use conclave_core::{Error, Result};

use crate::extract::ExtractedText;
use crate::Chunk;

use super::metadata::{DocumentRecord, DocumentStatus, MetadataStore};
use super::vector::{VectorHit, VectorStore};

/// Filesystem layout for a workspace's storage.
#[derive(Debug, Clone)]
pub struct RepositoryLayout {
    pub root: PathBuf,
}

impl RepositoryLayout {
    /// Build a layout rooted at a workspace directory.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            root: workspace_root.into(),
        }
    }

    /// Path to `metadata.sqlite`.
    pub fn metadata_db(&self) -> PathBuf {
        self.root.join("metadata.sqlite")
    }

    /// Path to the `vectors.lance/` dataset directory.
    pub fn vectors_dir(&self) -> PathBuf {
        self.root.join("vectors.lance")
    }

    /// Path to the `documents/` directory of file copies.
    pub fn documents_dir(&self) -> PathBuf {
        self.root.join("documents")
    }

    /// Create every subdirectory the repository needs.
    pub fn ensure_exists(&self) -> Result<()> {
        for dir in [&self.root, &self.documents_dir()] {
            std::fs::create_dir_all(dir).map_err(|e| Error::io_at(dir, e))?;
        }
        Ok(())
    }
}

/// Combined view over the per-workspace storage.
pub struct DocumentRepository {
    layout: RepositoryLayout,
    metadata: Mutex<MetadataStore>,
    vectors: VectorStore,
    embedding_dim: usize,
}

impl std::fmt::Debug for DocumentRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The metadata and vector handles intentionally elide their inner
        // state (rusqlite::Connection and lancedb::Connection don't impl
        // Debug); the layout + dim are enough for tracing-level diagnostics.
        f.debug_struct("DocumentRepository")
            .field("layout", &self.layout)
            .field("embedding_dim", &self.embedding_dim)
            .finish_non_exhaustive()
    }
}

impl DocumentRepository {
    /// Open (or create) the per-workspace stores under `layout`.
    pub async fn open(layout: RepositoryLayout, embedding_dim: usize) -> Result<Self> {
        layout.ensure_exists()?;
        let metadata = MetadataStore::open(layout.metadata_db())?;
        metadata.put_meta("embedding_dim", &embedding_dim.to_string())?;
        let vectors = VectorStore::open(layout.vectors_dir(), embedding_dim).await?;
        Ok(Self {
            layout,
            metadata: Mutex::new(metadata),
            vectors,
            embedding_dim,
        })
    }

    /// Embedding dimensionality this repository was opened with.
    pub const fn embedding_dim(&self) -> usize {
        self.embedding_dim
    }

    /// Compute the stable id and sha-256 that [`Self::add`] would assign to
    /// `path`. The caller can use the id to populate `Chunk.document_id`
    /// before chunks are passed to `add`.
    pub fn prepare_document_id(path: &Path) -> Result<(String, String)> {
        let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
        let sha = sha256_hex(&bytes);
        let id = build_document_id(&sha, path);
        Ok((id, sha))
    }

    /// Ingest a fully-processed document: copy its bytes into `documents/`,
    /// insert metadata, persist chunk vectors.
    ///
    /// The caller is expected to have computed `id` and `sha256` via
    /// [`Self::prepare_document_id`] so that the chunks coming in already
    /// reference the right `document_id`.
    ///
    /// On vector-store failure the `SQLite` rows are removed so subsequent
    /// retries don't see an orphaned half-state.
    pub async fn add(
        &self,
        id: &str,
        sha256: &str,
        extracted: &ExtractedText,
        chunks: &[Chunk],
        vectors: &[Vec<f32>],
    ) -> Result<DocumentRecord> {
        if chunks.len() != vectors.len() {
            return Err(Error::Rag(format!(
                "chunk/vector count mismatch: {} vs {}",
                chunks.len(),
                vectors.len()
            )));
        }
        let bytes = std::fs::read(&extracted.source_path)
            .map_err(|e| Error::io_at(&extracted.source_path, e))?;
        let copied_path = self.copy_into_documents_dir(id, &extracted.source_path, &bytes)?;
        let title = extracted
            .source_path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("")
            .to_owned();

        let status = if extracted.needs_ocr {
            DocumentStatus::NeedsOcr
        } else if chunks.is_empty() {
            DocumentStatus::Failed
        } else {
            DocumentStatus::Ready
        };

        let record = DocumentRecord {
            id: id.to_owned(),
            source_path: extracted.source_path.clone(),
            copied_path,
            title,
            doc_type: extracted.doc_type,
            sha256: sha256.to_owned(),
            ingested_at: Utc::now(),
            page_count: u32::try_from(extracted.page_breaks.len().saturating_add(1))
                .unwrap_or(u32::MAX),
            status,
        };

        // SQLite insert under its own lock so the connection's Sync constraint
        // (rusqlite::Connection is !Sync) doesn't bleed across .await points.
        {
            let metadata = self
                .metadata
                .lock()
                .map_err(|_| Error::Rag("metadata mutex poisoned".into()))?;
            metadata.insert_document(&record)?;
            metadata.insert_chunks(chunks)?;
        }

        if let Err(e) = self.vectors.upsert(chunks, vectors).await {
            // Compensate: remove the SQLite rows we just inserted so the
            // store doesn't carry an orphaned half-state.
            let undo = self
                .metadata
                .lock()
                .map_err(|_| Error::Rag("metadata mutex poisoned during rollback".into()))?
                .delete_document(id);
            tracing::error!(?undo, error = %e, "vector upsert failed; rolled back metadata rows");
            return Err(e);
        }

        Ok(record)
    }

    /// List every document.
    pub fn list(&self) -> Result<Vec<DocumentRecord>> {
        let metadata = self
            .metadata
            .lock()
            .map_err(|_| Error::Rag("metadata mutex poisoned".into()))?;
        metadata.list_documents()
    }

    /// Look up a single document and return its metadata + sample text.
    // Hold the lock across all three queries on purpose: they conceptually
    // form one snapshot.
    #[allow(clippy::significant_drop_tightening)]
    pub fn show(&self, id: &str) -> Result<Option<DocumentDetails>> {
        let metadata = self
            .metadata
            .lock()
            .map_err(|_| Error::Rag("metadata mutex poisoned".into()))?;
        let Some(record) = metadata.get_document(id)? else {
            return Ok(None);
        };
        let chunk_count = metadata.count_chunks(id)?;
        let sample = metadata.first_chunk_text(id)?;
        Ok(Some(DocumentDetails {
            record,
            chunk_count,
            sample_text: sample,
        }))
    }

    /// Remove a document and every artefact attached to it. Best-effort:
    /// vector deletion first, then metadata, then the file copy. Failures
    /// past metadata removal are logged but do not abort the call.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn remove(&self, id: &str) -> Result<bool> {
        // Vectors first: a half-deleted document loses retrievability but
        // not metadata, which we can fix on retry.
        self.vectors.delete_by_document(id).await?;

        // Capture the path before deleting the row so we can clean up the
        // file copy afterwards.
        // Hold the lock for the read+delete pair so a concurrent insert can
        // never reuse the id between our queries.
        #[allow(clippy::significant_drop_tightening)]
        let copy_path = {
            let metadata = self
                .metadata
                .lock()
                .map_err(|_| Error::Rag("metadata mutex poisoned".into()))?;
            let Some(record) = metadata.get_document(id)? else {
                return Ok(false);
            };
            let removed = metadata.delete_document(id)?;
            if !removed {
                return Ok(false);
            }
            record.copied_path
        };

        if let Err(e) = std::fs::remove_file(&copy_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(path = %copy_path.display(), error = %e, "could not remove file copy");
            }
        }
        Ok(true)
    }

    /// Top-K vector search.
    pub async fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        self.vectors.search(query, k).await
    }

    /// File-system layout this repository operates on.
    pub const fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn copy_into_documents_dir(&self, id: &str, source: &Path, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.layout.documents_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io_at(&dir, e))?;
        // The on-disk filename is `<id>.<ext>` — `id` already encodes the
        // sha prefix + a slug of the original stem, so we don't append the
        // original name again. Doing so could overflow the 255-byte
        // filename limit on macOS/Linux for documents with long titles.
        let ext = source
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("bin");
        let dest = dir.join(format!("{id}.{ext}"));
        std::fs::write(&dest, bytes).map_err(|e| Error::io_at(&dest, e))?;
        Ok(dest)
    }
}

/// Metadata + chunk-count + sample-text payload returned by [`DocumentRepository::show`].
#[derive(Debug, Clone)]
pub struct DocumentDetails {
    pub record: DocumentRecord,
    pub chunk_count: usize,
    pub sample_text: Option<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let mut s = String::with_capacity(result.len() * 2);
    for b in result {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Cap on the slug portion of a document id. macOS and most Linux
/// filesystems limit a single path component to 255 bytes; the id is
/// later interpolated into `documents/<id>.<ext>` and shown in URLs, so
/// keeping it short is also cosmetic. 60 chars is enough to keep a
/// human-recognisable hint while leaving headroom for path joining.
const SLUG_MAX_LEN: usize = 60;

fn build_document_id(sha_hex: &str, source: &Path) -> String {
    let prefix: String = sha_hex.chars().take(8).collect();
    let stem = source
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("document");
    let mut slug = slug::slugify(stem);
    if slug.len() > SLUG_MAX_LEN {
        slug.truncate(SLUG_MAX_LEN);
        // Drop a trailing hyphen that the truncation may have left behind
        // so the id never looks like `<sha>-…-` with a dangling dash.
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    if slug.is_empty() {
        prefix
    } else {
        format!("{prefix}-{slug}")
    }
}
