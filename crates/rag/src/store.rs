//! Persistent per-workspace knowledge store.
//!
//! Backed by a single `SQLite` file (`knowledge.sqlite` under the workspace
//! data directory). Layout:
//!
//! - `documents` — one row per ingested file. Identified by `path` (unique)
//!   and `content_hash` (Blake3 of the normalised text), which makes
//!   re-ingestion idempotent.
//! - `chunks` — one row per chunk produced by [`crate::chunking::chunk_text`].
//!   Carries the chunk text plus its L2-normalised embedding as a BLOB of
//!   `f32`s.
//! - `chunks_fts` — FTS5 virtual table mirroring chunk text + document title,
//!   used for BM25 retrieval (Unicode-aware tokenizer, diacritics folded).
//!
//! Dense retrieval uses a Rust scalar function `cosine_sim(blob, ?)`
//! registered on the connection. The blob layout is little-endian `f32`
//! values, identical to what a hypothetical `sqlite-vec` swap would store.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use conclave_core::{Error, Result};

use crate::loaders::DocumentFormat;

/// Persistent knowledge store for one workspace.
#[derive(Debug)]
pub struct KnowledgeStore {
    conn: Connection,
    dim: usize,
    path: PathBuf,
}

/// Aggregate statistics about the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    /// Number of documents currently in the store.
    pub documents: u64,
    /// Number of chunks across all documents.
    pub chunks: u64,
    /// Size of the underlying `SQLite` file in bytes.
    pub disk_bytes: u64,
}

/// A chunk retrieved from the store, with optional similarity score.
#[derive(Debug, Clone)]
pub struct StoredChunk {
    /// Internal chunk id.
    pub chunk_id: i64,
    /// Document id this chunk belongs to.
    pub document_id: i64,
    /// 0-based ordinal of this chunk within its document.
    pub chunk_index: i64,
    /// Source path of the document.
    pub path: PathBuf,
    /// Document title, if any.
    pub title: Option<String>,
    /// Document format.
    pub format: DocumentFormat,
    /// Chunk text content.
    pub text: String,
    /// Similarity score (whatever the calling query produced); `None` for
    /// non-search lookups.
    pub score: Option<f32>,
}

impl KnowledgeStore {
    /// Open (and create if missing) the store at `path` for a model with
    /// the given embedding dimension.
    ///
    /// The dimension is validated against the value stored in `meta` on
    /// subsequent opens: switching embedding models without re-ingesting
    /// is rejected, because the existing vectors would be meaningless.
    pub fn open(path: impl Into<PathBuf>, dim: usize) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        let conn = Connection::open(&path).map_err(map_sql)?;
        Self::configure_pragmas(&conn)?;
        Self::register_functions(&conn, dim)?;
        Self::create_schema(&conn)?;

        let store = Self { conn, dim, path };
        store.check_or_record_dim()?;
        Ok(store)
    }

    /// Open an in-memory store (used by tests).
    pub fn open_in_memory(dim: usize) -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_sql)?;
        Self::configure_pragmas(&conn)?;
        Self::register_functions(&conn, dim)?;
        Self::create_schema(&conn)?;
        let store = Self {
            conn,
            dim,
            path: PathBuf::from(":memory:"),
        };
        store.check_or_record_dim()?;
        Ok(store)
    }

    /// Embedding dimension this store was opened with.
    pub const fn dimension(&self) -> usize {
        self.dim
    }

    /// Path the store is backed by.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Look up a document by its source path.
    pub fn find_document_by_path(&self, path: &Path) -> Result<Option<DocumentRow>> {
        self.conn
            .query_row(
                "SELECT id, path, format, title, content_hash FROM documents WHERE path = ?1",
                params![path.to_string_lossy().as_ref()],
                DocumentRow::from_row,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Look up a document by its content hash.
    pub fn find_document_by_hash(&self, hash: &[u8; 32]) -> Result<Option<DocumentRow>> {
        self.conn
            .query_row(
                "SELECT id, path, format, title, content_hash FROM documents WHERE content_hash = ?1",
                params![hash.as_slice()],
                DocumentRow::from_row,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Insert a document together with its chunks and embeddings.
    ///
    /// All inserts run inside a single transaction; on error the store is
    /// left untouched. If a row with the same `path` already exists it is
    /// replaced (idempotent re-ingestion).
    pub fn upsert_document(&mut self, ingest: &DocumentInsert<'_>) -> Result<i64> {
        if ingest.chunks.len() != ingest.embeddings.len() {
            return Err(Error::Rag(format!(
                "chunk/embedding count mismatch: {} chunks, {} embeddings",
                ingest.chunks.len(),
                ingest.embeddings.len()
            )));
        }
        for v in ingest.embeddings {
            if v.len() != self.dim {
                return Err(Error::Rag(format!(
                    "embedding has {} dims, store expects {}",
                    v.len(),
                    self.dim
                )));
            }
        }

        let tx = self.conn.transaction().map_err(map_sql)?;

        // Replace any prior row for the same path: cascading FK delete drops
        // its chunks (and their FTS5 rows via triggers).
        tx.execute(
            "DELETE FROM documents WHERE path = ?1",
            params![ingest.path.to_string_lossy().as_ref()],
        )
        .map_err(map_sql)?;

        tx.execute(
            "INSERT INTO documents (path, format, title, content_hash, ingested_at, text_bytes, chunk_count) \
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'), ?5, ?6)",
            params![
                ingest.path.to_string_lossy().as_ref(),
                ingest.format.label(),
                ingest.title,
                ingest.content_hash.as_slice(),
                i64::try_from(ingest.text_bytes).unwrap_or(i64::MAX),
                i64::try_from(ingest.chunks.len()).unwrap_or(i64::MAX),
            ],
        )
        .map_err(map_sql)?;

        let doc_id = tx.last_insert_rowid();

        for (chunk, embedding) in ingest.chunks.iter().zip(ingest.embeddings.iter()) {
            tx.execute(
                "INSERT INTO chunks (doc_id, chunk_index, start_byte, end_byte, text, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    doc_id,
                    i64::try_from(chunk.index).unwrap_or(i64::MAX),
                    i64::try_from(chunk.start).unwrap_or(i64::MAX),
                    i64::try_from(chunk.end).unwrap_or(i64::MAX),
                    chunk.text,
                    encode_vector(embedding),
                ],
            )
            .map_err(map_sql)?;
            let chunk_row = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO chunks_fts (rowid, title, text) VALUES (?1, ?2, ?3)",
                params![chunk_row, ingest.title.unwrap_or(""), chunk.text],
            )
            .map_err(map_sql)?;
        }

        tx.commit().map_err(map_sql)?;
        Ok(doc_id)
    }

    /// Remove a document (and its chunks) by source path.
    ///
    /// Returns `true` if a row was deleted.
    pub fn remove_document(&self, path: &Path) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "DELETE FROM documents WHERE path = ?1",
                params![path.to_string_lossy().as_ref()],
            )
            .map_err(map_sql)?;
        Ok(n > 0)
    }

    /// Aggregate counts and disk footprint.
    pub fn stats(&self) -> Result<StoreStats> {
        let documents: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .map_err(map_sql)?;
        let chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .map_err(map_sql)?;
        let disk_bytes = if self.path.exists() {
            std::fs::metadata(&self.path)
                .map(|m| m.len())
                .unwrap_or_default()
        } else {
            0
        };
        Ok(StoreStats {
            documents: u64::try_from(documents).unwrap_or_default(),
            chunks: u64::try_from(chunks).unwrap_or_default(),
            disk_bytes,
        })
    }

    /// Dense (vector) search: returns chunks ranked by cosine similarity.
    pub fn search_dense(&self, query: &[f32], top_k: usize) -> Result<Vec<StoredChunk>> {
        if query.len() != self.dim {
            return Err(Error::Rag(format!(
                "query vector has {} dims, store expects {}",
                query.len(),
                self.dim
            )));
        }
        let blob = encode_vector(query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.doc_id, c.chunk_index, d.path, d.format, d.title, c.text, \
                        cosine_sim(c.embedding, ?1) AS score \
                 FROM chunks c JOIN documents d ON d.id = c.doc_id \
                 ORDER BY score DESC LIMIT ?2",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map(
                params![blob, i64::try_from(top_k).unwrap_or(i64::MAX)],
                StoredChunk::from_search_row,
            )
            .map_err(map_sql)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_sql)
    }

    /// BM25 search using FTS5.
    pub fn search_bm25(&self, query: &str, top_k: usize) -> Result<Vec<StoredChunk>> {
        let normalised = normalise_fts_query(query);
        if normalised.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.doc_id, c.chunk_index, d.path, d.format, d.title, c.text, \
                        bm25(chunks_fts) AS score \
                 FROM chunks_fts \
                 JOIN chunks c ON c.id = chunks_fts.rowid \
                 JOIN documents d ON d.id = c.doc_id \
                 WHERE chunks_fts MATCH ?1 \
                 ORDER BY bm25(chunks_fts) ASC LIMIT ?2",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map(
                params![normalised, i64::try_from(top_k).unwrap_or(i64::MAX)],
                |row| {
                    let mut hit = StoredChunk::from_search_row(row)?;
                    // FTS5's bm25() returns negative scores: lower is better.
                    // We negate so that callers can treat "higher = better"
                    // uniformly across modes (dense already does that).
                    hit.score = hit.score.map(|s| -s);
                    Ok(hit)
                },
            )
            .map_err(map_sql)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_sql)
    }

    // -- internals --------------------------------------------------------

    fn configure_pragmas(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(map_sql)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(map_sql)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_sql)?;
        Ok(())
    }

    fn register_functions(conn: &Connection, dim: usize) -> Result<()> {
        use rusqlite::functions::FunctionFlags;
        conn.create_scalar_function(
            "cosine_sim",
            2,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            move |ctx| {
                let a_blob = ctx.get_raw(0).as_blob().map_err(|e| {
                    rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(format!(
                        "cosine_sim arg 0: {e:?}"
                    ))))
                })?;
                let b_blob = ctx.get_raw(1).as_blob().map_err(|e| {
                    rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(format!(
                        "cosine_sim arg 1: {e:?}"
                    ))))
                })?;
                let a = decode_vector_borrowed(a_blob)?;
                let b = decode_vector_borrowed(b_blob)?;
                if a.len() != dim || b.len() != dim {
                    return Err(rusqlite::Error::UserFunctionError(Box::new(
                        std::io::Error::other(format!(
                            "cosine_sim dim mismatch: a={} b={} expected={}",
                            a.len(),
                            b.len(),
                            dim
                        )),
                    )));
                }
                Ok(f64::from(crate::embeddings::cosine_similarity(&a, &b)))
            },
        )
        .map_err(map_sql)?;
        Ok(())
    }

    fn create_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS documents (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                path          TEXT NOT NULL UNIQUE,
                format        TEXT NOT NULL,
                title         TEXT,
                content_hash  BLOB NOT NULL UNIQUE,
                ingested_at   INTEGER NOT NULL,
                text_bytes    INTEGER NOT NULL,
                chunk_count   INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS chunks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                doc_id      INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                start_byte  INTEGER NOT NULL,
                end_byte    INTEGER NOT NULL,
                text        TEXT NOT NULL,
                embedding   BLOB NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id);

             CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                title,
                text,
                tokenize='unicode61 remove_diacritics 2'
             );

             CREATE TRIGGER IF NOT EXISTS chunks_fts_after_delete
             AFTER DELETE ON chunks BEGIN
                DELETE FROM chunks_fts WHERE rowid = old.id;
             END;",
        )
        .map_err(map_sql)
    }

    fn check_or_record_dim(&self) -> Result<()> {
        let recorded: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_dim'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sql)?;
        match recorded {
            Some(v) => {
                let parsed: usize = v
                    .parse()
                    .map_err(|_| Error::Rag(format!("corrupted meta.embedding_dim value: {v}")))?;
                if parsed != self.dim {
                    return Err(Error::Rag(format!(
                        "store was created with embedding_dim={parsed}, but opened with {expected}; \
                         re-ingest is required",
                        expected = self.dim,
                    )));
                }
            }
            None => {
                self.conn
                    .execute(
                        "INSERT INTO meta(key, value) VALUES ('embedding_dim', ?1)",
                        params![self.dim.to_string()],
                    )
                    .map_err(map_sql)?;
            }
        }
        Ok(())
    }
}

/// One row of the `documents` table.
#[derive(Debug, Clone)]
pub struct DocumentRow {
    /// Internal document id.
    pub id: i64,
    /// Source path.
    pub path: PathBuf,
    /// Format string (matches [`DocumentFormat::label`]).
    pub format: String,
    /// Optional title.
    pub title: Option<String>,
    /// Blake3 content hash of the normalised text.
    pub content_hash: [u8; 32],
}

impl DocumentRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let path: String = row.get(1)?;
        let hash_blob: Vec<u8> = row.get(4)?;
        let mut hash = [0u8; 32];
        if hash_blob.len() == 32 {
            hash.copy_from_slice(&hash_blob);
        }
        Ok(Self {
            id: row.get(0)?,
            path: PathBuf::from(path),
            format: row.get(2)?,
            title: row.get(3)?,
            content_hash: hash,
        })
    }
}

/// Input for [`KnowledgeStore::upsert_document`].
#[derive(Debug)]
pub struct DocumentInsert<'a> {
    /// Source path of the document.
    pub path: &'a Path,
    /// Detected format.
    pub format: DocumentFormat,
    /// Optional title.
    pub title: Option<&'a str>,
    /// Length of the normalised text in bytes (informational).
    pub text_bytes: usize,
    /// Blake3 hash of the normalised text.
    pub content_hash: [u8; 32],
    /// Chunks produced by [`crate::chunking::chunk_text`].
    pub chunks: &'a [crate::chunking::Chunk],
    /// One embedding per chunk, in the same order.
    pub embeddings: &'a [Vec<f32>],
}

impl StoredChunk {
    fn from_search_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let path: String = row.get(3)?;
        let format: String = row.get(4)?;
        Ok(Self {
            chunk_id: row.get(0)?,
            document_id: row.get(1)?,
            chunk_index: row.get(2)?,
            path: PathBuf::from(path),
            format: parse_format(&format),
            title: row.get(5)?,
            text: row.get(6)?,
            score: row.get(7)?,
        })
    }
}

fn parse_format(label: &str) -> DocumentFormat {
    match label {
        "markdown" => DocumentFormat::Markdown,
        "pdf" => DocumentFormat::Pdf,
        "html" => DocumentFormat::Html,
        "docx" => DocumentFormat::Docx,
        // "text" or any unknown label — we control inserts so the wildcard
        // should never fire in practice.
        _ => DocumentFormat::PlainText,
    }
}

fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn decode_vector_borrowed(blob: &[u8]) -> rusqlite::Result<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return Err(rusqlite::Error::UserFunctionError(Box::new(
            std::io::Error::other(format!(
                "embedding blob length {} is not a multiple of 4",
                blob.len()
            )),
        )));
    }
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

/// Sanitise free-form text into a safe FTS5 MATCH expression.
///
/// We use a plain prefix-match phrase per token: this preserves recall for
/// stemmed clinical vocabulary without forcing the caller to escape FTS5
/// metacharacters.
fn normalise_fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', " ")))
        .collect();
    tokens.join(" OR ")
}

fn map_sql(e: rusqlite::Error) -> Error {
    Error::Rag(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{chunk_text, ChunkParams};
    use crate::embeddings::{Embedder, MockEmbedder};

    fn hash_text(s: &str) -> [u8; 32] {
        *blake3::hash(s.as_bytes()).as_bytes()
    }

    fn ingest_synthetic(store: &mut KnowledgeStore, embedder: &MockEmbedder) {
        let cases = [
            (
                "/fake/cardio.md",
                DocumentFormat::Markdown,
                "Manejo del IAMCEST",
                "Reperfusión primaria con angioplastia antes de 120 minutos. \
                 Antiagregación con AAS 300 mg y prasugrel 60 mg.",
            ),
            (
                "/fake/neuro.md",
                DocumentFormat::Markdown,
                "Ictus isquémico agudo",
                "Trombólisis intravenosa con alteplasa en ventana <4.5 h. \
                 Trombectomía mecánica si oclusión de gran vaso.",
            ),
        ];
        let params = ChunkParams::new(120, 16).unwrap();
        for (path, format, title, text) in cases {
            let chunks = chunk_text(text, params);
            let chunk_refs: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = embedder.embed(&chunk_refs).unwrap();
            let insert = DocumentInsert {
                path: Path::new(path),
                format,
                title: Some(title),
                text_bytes: text.len(),
                content_hash: hash_text(text),
                chunks: &chunks,
                embeddings: &embeddings,
            };
            store.upsert_document(&insert).unwrap();
        }
    }

    #[test]
    fn upsert_and_stats() {
        let embedder = MockEmbedder::new(48);
        let mut store = KnowledgeStore::open_in_memory(48).unwrap();
        ingest_synthetic(&mut store, &embedder);
        let stats = store.stats().unwrap();
        assert_eq!(stats.documents, 2);
        assert!(stats.chunks >= 2);
    }

    #[test]
    fn upsert_is_idempotent_per_path() {
        let embedder = MockEmbedder::new(48);
        let mut store = KnowledgeStore::open_in_memory(48).unwrap();
        ingest_synthetic(&mut store, &embedder);
        let before = store.stats().unwrap();
        ingest_synthetic(&mut store, &embedder);
        let after = store.stats().unwrap();
        assert_eq!(before, after, "re-ingestion should be a no-op");
    }

    #[test]
    fn dense_search_returns_closest_doc() {
        let embedder = MockEmbedder::new(96);
        let mut store = KnowledgeStore::open_in_memory(96).unwrap();
        ingest_synthetic(&mut store, &embedder);

        let query = embedder
            .embed_one("reperfusión angioplastia primaria")
            .unwrap();
        let hits = store.search_dense(&query, 5).unwrap();
        assert!(!hits.is_empty());
        let top = &hits[0];
        assert!(top.title.as_deref() == Some("Manejo del IAMCEST"));
    }

    #[test]
    fn bm25_search_finds_terms() {
        let embedder = MockEmbedder::new(48);
        let mut store = KnowledgeStore::open_in_memory(48).unwrap();
        ingest_synthetic(&mut store, &embedder);

        let hits = store.search_bm25("alteplasa", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].title.as_deref(), Some("Ictus isquémico agudo"));
    }

    #[test]
    fn remove_document_deletes_chunks() {
        let embedder = MockEmbedder::new(48);
        let mut store = KnowledgeStore::open_in_memory(48).unwrap();
        ingest_synthetic(&mut store, &embedder);
        assert!(store.remove_document(Path::new("/fake/cardio.md")).unwrap());
        let stats = store.stats().unwrap();
        assert_eq!(stats.documents, 1);
    }

    #[test]
    fn rejects_dim_mismatch_on_insert() {
        let mut store = KnowledgeStore::open_in_memory(48).unwrap();
        let chunks = chunk_text("hola mundo", ChunkParams::new(8, 1).unwrap());
        let bad = vec![vec![0.0f32; 47]; chunks.len()];
        let insert = DocumentInsert {
            path: Path::new("/x.md"),
            format: DocumentFormat::Markdown,
            title: None,
            text_bytes: 10,
            content_hash: [0u8; 32],
            chunks: &chunks,
            embeddings: &bad,
        };
        assert!(store.upsert_document(&insert).is_err());
    }
}
