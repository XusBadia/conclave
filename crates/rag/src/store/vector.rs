//! LanceDB-backed vector store.
//!
//! Owns the `vectors.lance/` dataset inside a workspace directory. Holds one
//! table named `chunks` keyed by chunk id, with columns:
//!
//! - `id` (utf8)
//! - `document_id` (utf8)
//! - `text` (utf8)
//! - `embedding` (fixed-size list of f32, length = embedding dim)
//!
//! All operations are async because `lancedb` is.

use std::path::Path;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{Connection, Table};

use conclave_core::{Error, Result};

use crate::Chunk;

const TABLE_NAME: &str = "chunks";

/// LanceDB-side of the per-workspace storage.
pub struct VectorStore {
    conn: Connection,
    dim: usize,
}

impl std::fmt::Debug for VectorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `lancedb::Connection` does not implement Debug; we surface only
        // the dimensionality, which is the only field worth tracing.
        f.debug_struct("VectorStore")
            .field("dim", &self.dim)
            .finish_non_exhaustive()
    }
}

/// One row returned by a vector search.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    /// Chunk id.
    pub chunk_id: String,
    /// Owning document id.
    pub document_id: String,
    /// Chunk text (copy of the `SQLite` column).
    pub text: String,
    /// `L2` distance reported by `lancedb`. Lower is closer.
    pub distance: f32,
}

impl VectorStore {
    /// Open or create the dataset at `path` for vectors of dimension `dim`.
    pub async fn open(path: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let uri = path.as_ref().to_string_lossy().to_string();
        let conn = lancedb::connect(&uri)
            .execute()
            .await
            .map_err(|e| Error::Rag(format!("lancedb connect {uri}: {e}")))?;
        Ok(Self { conn, dim })
    }

    /// Embedding dimensionality this store expects on every input.
    pub const fn dim(&self) -> usize {
        self.dim
    }

    /// Append a batch of chunks + embeddings. Creates the table on first
    /// call. Mismatched lengths or wrong-dim vectors are rejected.
    pub async fn upsert(&self, chunks: &[Chunk], vectors: &[Vec<f32>]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        if chunks.len() != vectors.len() {
            return Err(Error::Rag(format!(
                "chunk/vector count mismatch: {} chunks vs {} vectors",
                chunks.len(),
                vectors.len()
            )));
        }
        for v in vectors {
            if v.len() != self.dim {
                return Err(Error::Rag(format!(
                    "vector dim mismatch: expected {}, got {}",
                    self.dim,
                    v.len()
                )));
            }
        }
        let schema = self.schema();
        let batch = build_batch(&schema, self.dim, chunks, vectors)?;

        if self.table_exists().await? {
            let table = self.open_table().await?;
            let iter = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
            let reader: Box<dyn RecordBatchReader + Send> = Box::new(iter);
            table
                .add(reader)
                .execute()
                .await
                .map_err(|e| Error::Rag(format!("lancedb add: {e}")))?;
        } else {
            let iter = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
            let reader: Box<dyn RecordBatchReader + Send> = Box::new(iter);
            self.conn
                .create_table(TABLE_NAME, reader)
                .execute()
                .await
                .map_err(|e| Error::Rag(format!("lancedb create_table: {e}")))?;
        }
        Ok(())
    }

    /// Top-K nearest-neighbour search. Returns an empty `Vec` if the table
    /// has not been created yet.
    pub async fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        if query.len() != self.dim {
            return Err(Error::Rag(format!(
                "query dim mismatch: expected {}, got {}",
                self.dim,
                query.len()
            )));
        }
        if !self.table_exists().await? {
            return Ok(Vec::new());
        }
        let table = self.open_table().await?;
        let mut stream = table
            .query()
            .nearest_to(query.to_vec())
            .map_err(|e| Error::Rag(format!("lancedb nearest_to: {e}")))?
            .limit(k)
            .execute()
            .await
            .map_err(|e| Error::Rag(format!("lancedb query execute: {e}")))?;

        let mut hits = Vec::new();
        while let Some(batch) = stream
            .try_next()
            .await
            .map_err(|e| Error::Rag(format!("lancedb stream: {e}")))?
        {
            extend_hits(&batch, &mut hits)?;
        }
        Ok(hits)
    }

    /// Remove every row belonging to a document. Idempotent.
    pub async fn delete_by_document(&self, document_id: &str) -> Result<()> {
        if !self.table_exists().await? {
            return Ok(());
        }
        let table = self.open_table().await?;
        // `document_id` is user-controlled at higher levels; escape single
        // quotes to keep the predicate well-formed.
        let escaped = document_id.replace('\'', "''");
        let predicate = format!("document_id = '{escaped}'");
        table
            .delete(&predicate)
            .await
            .map_err(|e| Error::Rag(format!("lancedb delete: {e}")))?;
        Ok(())
    }

    async fn table_exists(&self) -> Result<bool> {
        let names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| Error::Rag(format!("lancedb table_names: {e}")))?;
        Ok(names.iter().any(|n| n == TABLE_NAME))
    }

    async fn open_table(&self) -> Result<Table> {
        self.conn
            .open_table(TABLE_NAME)
            .execute()
            .await
            .map_err(|e| Error::Rag(format!("lancedb open_table: {e}")))
    }

    fn schema(&self) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("document_id", DataType::Utf8, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "embedding",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    i32::try_from(self.dim).unwrap_or(i32::MAX),
                ),
                false,
            ),
        ]))
    }
}

fn build_batch(
    schema: &Arc<Schema>,
    dim: usize,
    chunks: &[Chunk],
    vectors: &[Vec<f32>],
) -> Result<RecordBatch> {
    let ids: StringArray = chunks.iter().map(|c| Some(c.id.as_str())).collect();
    let doc_ids: StringArray = chunks
        .iter()
        .map(|c| Some(c.document_id.as_str()))
        .collect();
    let texts: StringArray = chunks.iter().map(|c| Some(c.text.as_str())).collect();

    let flat: Vec<f32> = vectors.iter().flat_map(|v| v.iter().copied()).collect();
    let values = Arc::new(Float32Array::from(flat));
    let item_field = Arc::new(Field::new("item", DataType::Float32, true));
    let dim_i32 = i32::try_from(dim).map_err(|_| Error::Rag("dim does not fit in i32".into()))?;
    let emb = FixedSizeListArray::try_new(item_field, dim_i32, values, None)
        .map_err(|e| Error::Rag(format!("arrow FixedSizeListArray: {e}")))?;

    RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ids),
            Arc::new(doc_ids),
            Arc::new(texts),
            Arc::new(emb),
        ],
    )
    .map_err(|e| Error::Rag(format!("arrow RecordBatch: {e}")))
}

fn extend_hits(batch: &RecordBatch, out: &mut Vec<VectorHit>) -> Result<()> {
    let ids = batch
        .column_by_name("id")
        .ok_or_else(|| Error::Rag("vector result missing `id`".into()))?
        .as_string_opt::<i32>()
        .ok_or_else(|| Error::Rag("vector `id` is not utf8".into()))?;
    let doc_ids = batch
        .column_by_name("document_id")
        .ok_or_else(|| Error::Rag("vector result missing `document_id`".into()))?
        .as_string_opt::<i32>()
        .ok_or_else(|| Error::Rag("vector `document_id` is not utf8".into()))?;
    let texts = batch
        .column_by_name("text")
        .ok_or_else(|| Error::Rag("vector result missing `text`".into()))?
        .as_string_opt::<i32>()
        .ok_or_else(|| Error::Rag("vector `text` is not utf8".into()))?;
    let distances = batch
        .column_by_name("_distance")
        .ok_or_else(|| Error::Rag("vector result missing `_distance`".into()))?
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| Error::Rag("vector `_distance` is not float32".into()))?;

    for i in 0..batch.num_rows() {
        out.push(VectorHit {
            chunk_id: ids.value(i).to_owned(),
            document_id: doc_ids.value(i).to_owned(),
            text: texts.value(i).to_owned(),
            distance: distances.value(i),
        });
    }
    Ok(())
}
