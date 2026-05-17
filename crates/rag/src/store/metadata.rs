//! SQLite-backed metadata store.
//!
//! Owns the `metadata.sqlite` file inside a workspace directory. Holds
//! documents, chunks, tags and a small `schema_meta` key/value table for
//! bookkeeping (schema version, embedding dim).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

use crate::extract::DocType;
use crate::Chunk;

const SCHEMA_VERSION: u32 = 1;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    source_path TEXT NOT NULL,
    copied_path TEXT NOT NULL,
    title TEXT NOT NULL,
    doc_type TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    ingested_at TEXT NOT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'ready'
);

CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    page_start INTEGER NOT NULL DEFAULT 0,
    page_end INTEGER NOT NULL DEFAULT 0,
    text TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS chunks_document_id_idx ON chunks(document_id);

CREATE TABLE IF NOT EXISTS tags (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (document_id, tag)
);

CREATE TABLE IF NOT EXISTS schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Row in the `documents` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRecord {
    /// Stable id derived from the file's sha256 prefix and slugified name.
    pub id: String,
    /// Path to the file as the user supplied it.
    pub source_path: PathBuf,
    /// Path to our internal copy of the bytes.
    pub copied_path: PathBuf,
    /// Display title (defaults to file stem).
    pub title: String,
    /// Detected document type.
    pub doc_type: DocType,
    /// SHA-256 of the original bytes (hex).
    pub sha256: String,
    /// Timestamp when ingestion was completed.
    pub ingested_at: DateTime<Utc>,
    /// Page count (best-effort — 0 when unknown).
    pub page_count: u32,
    /// Ingestion outcome flag.
    pub status: DocumentStatus,
}

/// Outcome of the ingestion attempt for a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Text extracted and chunks embedded.
    Ready,
    /// Text extraction returned empty — OCR is required.
    NeedsOcr,
    /// Extraction errored out; the document is in the store but unsearchable.
    Failed,
}

impl DocumentStatus {
    const fn as_db_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsOcr => "needs_ocr",
            Self::Failed => "failed",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ready" => Some(Self::Ready),
            "needs_ocr" => Some(Self::NeedsOcr),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

const fn doc_type_as_db_str(d: DocType) -> &'static str {
    match d {
        DocType::Pdf => "pdf",
        DocType::Docx => "docx",
        DocType::Txt => "txt",
        DocType::Md => "md",
        DocType::Html => "html",
        DocType::Image => "image",
    }
}

fn doc_type_from_db_str(s: &str) -> Option<DocType> {
    match s {
        "pdf" => Some(DocType::Pdf),
        "docx" => Some(DocType::Docx),
        "txt" => Some(DocType::Txt),
        "md" => Some(DocType::Md),
        "html" => Some(DocType::Html),
        "image" => Some(DocType::Image),
        _ => None,
    }
}

/// Metadata-side of the per-workspace storage.
#[derive(Debug)]
pub struct MetadataStore {
    conn: Connection,
}

impl MetadataStore {
    /// Open or create `metadata.sqlite` at `path` and apply pending
    /// migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref()).map_err(map_sql)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_sql)?;
        conn.execute_batch(SCHEMA_SQL).map_err(map_sql)?;
        let store = Self { conn };
        store.put_meta("version", &SCHEMA_VERSION.to_string())?;
        Ok(store)
    }

    /// Persist a (key, value) pair in `schema_meta`. Idempotent.
    pub fn put_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Look up a `schema_meta` value.
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql)
    }

    /// Convenience accessor for the embedding dimension.
    pub fn embedding_dim(&self) -> Result<Option<usize>> {
        Ok(self.get_meta("embedding_dim")?.and_then(|s| s.parse().ok()))
    }

    /// Insert a document row. Caller must ensure `id` is unique.
    pub fn insert_document(&self, doc: &DocumentRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO documents
                   (id, source_path, copied_path, title, doc_type, sha256,
                    ingested_at, page_count, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    doc.id,
                    doc.source_path.to_string_lossy().to_string(),
                    doc.copied_path.to_string_lossy().to_string(),
                    doc.title,
                    doc_type_as_db_str(doc.doc_type),
                    doc.sha256,
                    doc.ingested_at.to_rfc3339(),
                    i64::from(doc.page_count),
                    doc.status.as_db_str(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert chunks belonging to a previously-inserted document. Wrapped in
    /// a single `SQLite` transaction.
    pub fn insert_chunks(&self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction().map_err(map_sql)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO chunks
                       (id, document_id, position, page_start, page_end, text)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(map_sql)?;
            for c in chunks {
                stmt.execute(params![
                    c.id,
                    c.document_id,
                    i64::from(c.position),
                    i64::from(c.page_start),
                    i64::from(c.page_end),
                    c.text,
                ])
                .map_err(map_sql)?;
            }
        }
        tx.commit().map_err(map_sql)?;
        Ok(())
    }

    /// List every document in ingestion-time-descending order.
    pub fn list_documents(&self) -> Result<Vec<DocumentRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source_path, copied_path, title, doc_type, sha256,
                        ingested_at, page_count, status
                 FROM documents
                 ORDER BY ingested_at DESC, id ASC",
            )
            .map_err(map_sql)?;
        let mut out = Vec::new();
        let rows = stmt.query_map([], row_to_raw).map_err(map_sql)?;
        for row in rows {
            out.push(row.map_err(map_sql)?.into_record()?);
        }
        Ok(out)
    }

    /// Find a single document by exact id.
    pub fn get_document(&self, id: &str) -> Result<Option<DocumentRecord>> {
        let raw = self
            .conn
            .query_row(
                "SELECT id, source_path, copied_path, title, doc_type, sha256,
                        ingested_at, page_count, status
                 FROM documents WHERE id = ?1",
                params![id],
                row_to_raw,
            )
            .optional()
            .map_err(map_sql)?;
        match raw {
            Some(r) => Ok(Some(r.into_record()?)),
            None => Ok(None),
        }
    }

    /// Count chunks belonging to a document.
    pub fn count_chunks(&self, document_id: &str) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE document_id = ?1",
                params![document_id],
                |r| r.get(0),
            )
            .map_err(map_sql)?;
        Ok(usize::try_from(count.max(0)).unwrap_or(0))
    }

    /// First chunk's text (used by `documents show`).
    pub fn first_chunk_text(&self, document_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT text FROM chunks WHERE document_id = ?1 ORDER BY position LIMIT 1",
                params![document_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql)
    }

    /// Delete a document, its chunks (via cascade) and its tags. Returns
    /// `true` when a row was actually deleted.
    pub fn delete_document(&self, id: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM documents WHERE id = ?1", params![id])
            .map_err(map_sql)?;
        Ok(n > 0)
    }
}

#[derive(Debug)]
struct RawRow {
    id: String,
    source_path: String,
    copied_path: String,
    title: String,
    doc_type: String,
    sha256: String,
    ingested_at: String,
    page_count: i64,
    status: String,
}

fn row_to_raw(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: r.get(0)?,
        source_path: r.get(1)?,
        copied_path: r.get(2)?,
        title: r.get(3)?,
        doc_type: r.get(4)?,
        sha256: r.get(5)?,
        ingested_at: r.get(6)?,
        page_count: r.get(7)?,
        status: r.get(8)?,
    })
}

impl RawRow {
    fn into_record(self) -> Result<DocumentRecord> {
        Ok(DocumentRecord {
            id: self.id,
            source_path: PathBuf::from(self.source_path),
            copied_path: PathBuf::from(self.copied_path),
            title: self.title,
            doc_type: doc_type_from_db_str(&self.doc_type)
                .ok_or_else(|| Error::Rag(format!("unknown doc_type: {}", self.doc_type)))?,
            sha256: self.sha256,
            ingested_at: DateTime::parse_from_rfc3339(&self.ingested_at)
                .map_err(|e| Error::Rag(format!("bad ingested_at: {e}")))?
                .with_timezone(&Utc),
            page_count: u32::try_from(self.page_count.max(0)).unwrap_or(u32::MAX),
            status: DocumentStatus::from_db_str(&self.status)
                .ok_or_else(|| Error::Rag(format!("unknown status: {}", self.status)))?,
        })
    }
}

fn map_sql(e: rusqlite::Error) -> Error {
    Error::Rag(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc(id: &str) -> DocumentRecord {
        DocumentRecord {
            id: id.to_owned(),
            source_path: PathBuf::from("/tmp/source.txt"),
            copied_path: PathBuf::from("/tmp/copied.txt"),
            title: "Sample".into(),
            doc_type: DocType::Txt,
            sha256: "abc123".into(),
            ingested_at: Utc::now(),
            page_count: 1,
            status: DocumentStatus::Ready,
        }
    }

    fn sample_chunk(id: &str, doc_id: &str, position: u32) -> Chunk {
        Chunk {
            id: id.to_owned(),
            text: format!("chunk {position}"),
            document_id: doc_id.to_owned(),
            page_start: 0,
            page_end: 0,
            position,
        }
    }

    #[test]
    fn open_creates_schema_and_records_version() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        assert_eq!(store.get_meta("version").unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn insert_then_get_document() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        let doc = sample_doc("doc-1");
        store.insert_document(&doc).unwrap();
        let got = store.get_document("doc-1").unwrap().unwrap();
        assert_eq!(got.id, doc.id);
        assert_eq!(got.title, doc.title);
        assert_eq!(got.doc_type, DocType::Txt);
        assert_eq!(got.status, DocumentStatus::Ready);
    }

    #[test]
    fn insert_chunks_then_count() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        store.insert_document(&sample_doc("doc-1")).unwrap();
        store
            .insert_chunks(&[
                sample_chunk("doc-1-0", "doc-1", 0),
                sample_chunk("doc-1-1", "doc-1", 1),
            ])
            .unwrap();
        assert_eq!(store.count_chunks("doc-1").unwrap(), 2);
        let first = store.first_chunk_text("doc-1").unwrap().unwrap();
        assert_eq!(first, "chunk 0");
    }

    #[test]
    fn delete_cascades_to_chunks() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        store.insert_document(&sample_doc("doc-1")).unwrap();
        store
            .insert_chunks(&[sample_chunk("doc-1-0", "doc-1", 0)])
            .unwrap();
        assert_eq!(store.count_chunks("doc-1").unwrap(), 1);
        let removed = store.delete_document("doc-1").unwrap();
        assert!(removed);
        assert_eq!(store.count_chunks("doc-1").unwrap(), 0);
        assert!(store.get_document("doc-1").unwrap().is_none());
    }

    #[test]
    fn list_documents_returns_inserted_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        store.insert_document(&sample_doc("doc-1")).unwrap();
        store.insert_document(&sample_doc("doc-2")).unwrap();
        let listed = store.list_documents().unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn embedding_dim_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(tmp.path().join("metadata.sqlite")).unwrap();
        assert_eq!(store.embedding_dim().unwrap(), None);
        store.put_meta("embedding_dim", "384").unwrap();
        assert_eq!(store.embedding_dim().unwrap(), Some(384));
    }
}
