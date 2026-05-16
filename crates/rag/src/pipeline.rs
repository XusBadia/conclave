//! Ingestion pipeline: extract → chunk → embed → store.
//!
//! Orchestrates the per-document flow and reports progress through a
//! caller-supplied closure. Operates on a single file or recursively on a
//! directory tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use conclave_core::{Error, Result};

use crate::chunk::{chunk_text, ChunkParams};
use crate::embed::Embedder;
use crate::extract::{extract_from_path, DocType};
use crate::store::{DocumentRecord, DocumentRepository};

/// Drives extraction, chunking, embedding and persistence end-to-end.
pub struct IngestionPipeline {
    embedder: Arc<dyn Embedder>,
    repository: Arc<DocumentRepository>,
    chunk_params: ChunkParams,
}

impl std::fmt::Debug for IngestionPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestionPipeline")
            .field("embedder", &self.embedder.id())
            .field("chunk_params", &self.chunk_params)
            .finish_non_exhaustive()
    }
}

/// Summary of a single `ingest_path` call.
#[derive(Debug, Default, Clone)]
pub struct IngestionReport {
    /// Documents that landed in the store.
    pub ingested: Vec<DocumentRecord>,
    /// Documents that were intentionally skipped (e.g. OCR not enabled, or
    /// the file type is unsupported).
    pub skipped: Vec<SkippedDocument>,
    /// Documents whose ingestion errored out. Other documents still run.
    pub failed: Vec<FailedDocument>,
}

/// A document we walked past without ingesting.
#[derive(Debug, Clone)]
pub struct SkippedDocument {
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// Why a document was skipped.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// The file's extension is not recognised by the extractor dispatcher.
    UnsupportedType,
    /// `pdf-extract` and `lopdf` both returned empty text — needs OCR.
    NeedsOcr,
}

/// A document that errored mid-ingest.
#[derive(Debug, Clone)]
pub struct FailedDocument {
    pub path: PathBuf,
    pub error: String,
}

/// Streaming progress event surfaced to the caller (CLI, UI).
#[derive(Debug, Clone)]
pub enum IngestionEvent {
    /// A document was discovered and ingestion is about to start.
    Starting(PathBuf),
    /// Ingestion finished successfully.
    Ingested {
        path: PathBuf,
        record: Box<DocumentRecord>,
    },
    /// The document was skipped (e.g. OCR not enabled).
    Skipped { path: PathBuf, reason: SkipReason },
    /// Ingestion errored out for this document; the run continues.
    Failed { path: PathBuf, error: String },
}

impl IngestionPipeline {
    /// Build a pipeline over an embedder + repository pair, with custom
    /// chunking parameters.
    pub fn new(
        embedder: Arc<dyn Embedder>,
        repository: Arc<DocumentRepository>,
        chunk_params: ChunkParams,
    ) -> Result<Self> {
        if embedder.dim() != repository.embedding_dim() {
            return Err(Error::Rag(format!(
                "embedder/repository dim mismatch: embedder={}, repository={}",
                embedder.dim(),
                repository.embedding_dim()
            )));
        }
        Ok(Self {
            embedder,
            repository,
            chunk_params,
        })
    }

    /// Access the underlying repository — primarily for callers that need
    /// to issue searches or list/remove documents against the same workspace.
    pub const fn repository(&self) -> &Arc<DocumentRepository> {
        &self.repository
    }

    /// Embedder driving this pipeline. Useful when callers want to embed
    /// query strings with the same backend.
    pub const fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    /// Process `path` (a single file or a directory walked recursively),
    /// streaming events to `on_event`. Returns an aggregate report.
    pub async fn ingest_path<F>(&self, path: &Path, mut on_event: F) -> Result<IngestionReport>
    where
        F: FnMut(IngestionEvent),
    {
        let files = collect_files(path)?;
        let mut report = IngestionReport::default();

        for file in files {
            on_event(IngestionEvent::Starting(file.clone()));
            if DocType::from_path(&file).is_none() {
                let skipped = SkippedDocument {
                    path: file.clone(),
                    reason: SkipReason::UnsupportedType,
                };
                on_event(IngestionEvent::Skipped {
                    path: file,
                    reason: skipped.reason.clone(),
                });
                report.skipped.push(skipped);
                continue;
            }
            match self.ingest_one(&file).await {
                Ok(Outcome::Ingested(record)) => {
                    on_event(IngestionEvent::Ingested {
                        path: file,
                        record: Box::new(record.clone()),
                    });
                    report.ingested.push(record);
                }
                Ok(Outcome::Skipped(reason)) => {
                    on_event(IngestionEvent::Skipped {
                        path: file.clone(),
                        reason: reason.clone(),
                    });
                    report.skipped.push(SkippedDocument { path: file, reason });
                }
                Err(e) => {
                    let msg = e.to_string();
                    on_event(IngestionEvent::Failed {
                        path: file.clone(),
                        error: msg.clone(),
                    });
                    report.failed.push(FailedDocument {
                        path: file,
                        error: msg,
                    });
                }
            }
        }

        Ok(report)
    }

    async fn ingest_one(&self, path: &Path) -> Result<Outcome> {
        let extracted = extract_from_path(path)?;
        if extracted.needs_ocr {
            return Ok(Outcome::Skipped(SkipReason::NeedsOcr));
        }

        let (id, sha) = DocumentRepository::prepare_document_id(path)?;
        let chunks = chunk_text(&extracted.content, &id, self.chunk_params)?;

        // Embed chunk texts. The embedder is sync; offload to spawn_blocking
        // so we don't block the tokio runtime if the FastEmbed backend is
        // doing real ONNX inference.
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embedder = Arc::clone(&self.embedder);
        let vectors = tokio::task::spawn_blocking(move || embedder.embed(&texts))
            .await
            .map_err(|e| Error::Rag(format!("embed task join: {e}")))??;

        let record = self
            .repository
            .add(&id, &sha, &extracted, &chunks, &vectors)
            .await?;
        Ok(Outcome::Ingested(record))
    }
}

enum Outcome {
    Ingested(DocumentRecord),
    Skipped(SkipReason),
}

fn collect_files(path: &Path) -> Result<Vec<PathBuf>> {
    let meta = std::fs::metadata(path).map_err(|e| Error::io_at(path, e))?;
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if meta.is_dir() {
        let mut out = Vec::new();
        walk_dir(path, &mut out)?;
        out.sort();
        return Ok(out);
    }
    Err(Error::Rag(format!(
        "path is neither a file nor a directory: {}",
        path.display()
    )))
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io_at(dir, e))? {
        let entry = entry.map_err(|e| Error::io_at(dir, e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| Error::io_at(&path, e))?;
        if file_type.is_dir() {
            walk_dir(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;

    use crate::embed::MockEmbedder;
    use crate::store::RepositoryLayout;

    fn write_fixture(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    async fn fresh_pipeline(tmp: &tempfile::TempDir) -> IngestionPipeline {
        let layout = RepositoryLayout::new(tmp.path().join("workspace"));
        let repo = Arc::new(
            DocumentRepository::open(layout, MockEmbedder::new().dim())
                .await
                .unwrap(),
        );
        IngestionPipeline::new(Arc::new(MockEmbedder::new()), repo, ChunkParams::DEFAULT).unwrap()
    }

    #[tokio::test]
    async fn ingest_single_text_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_fixture(
            tmp.path(),
            "notes.txt",
            "La insuficiencia cardiaca aguda requiere diuréticos. \
             El paciente fue tratado con furosemida intravenosa.",
        );
        let pipeline = fresh_pipeline(&tmp).await;

        let mut events = Vec::new();
        let report = pipeline
            .ingest_path(&file, |e| events.push(format!("{e:?}")))
            .await
            .unwrap();
        assert_eq!(report.ingested.len(), 1);
        assert!(report.skipped.is_empty());
        assert!(report.failed.is_empty());
        let record = &report.ingested[0];
        assert_eq!(record.title, "notes");
        assert!(events.iter().any(|s| s.contains("Starting")));
        assert!(events.iter().any(|s| s.contains("Ingested")));
    }

    #[tokio::test]
    async fn ingest_directory_recurses() {
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("docs");
        std::fs::create_dir_all(&subdir).unwrap();
        write_fixture(
            &subdir,
            "a.txt",
            "Primer documento sobre diabetes mellitus tipo 2.",
        );
        write_fixture(&subdir, "b.md", "# Hipertensión\n\nNotas clínicas básicas.");
        write_fixture(&subdir, "c.xyz", "ignored: unsupported extension");

        let pipeline = fresh_pipeline(&tmp).await;
        let report = pipeline.ingest_path(&subdir, |_| {}).await.unwrap();
        assert_eq!(report.ingested.len(), 2);
        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(
            report.skipped[0].reason,
            SkipReason::UnsupportedType
        ));
    }

    #[tokio::test]
    async fn search_finds_known_chunk() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_fixture(
            tmp.path(),
            "cardio.txt",
            "Sample about cardiology including infarction and arrhythmia.",
        );
        let pipeline = fresh_pipeline(&tmp).await;
        pipeline.ingest_path(&file, |_| {}).await.unwrap();

        let query_vec = pipeline
            .embedder
            .embed(&["cardiology infarction".to_string()])
            .unwrap();
        let hits = pipeline.repository.search(&query_vec[0], 5).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        // The mock embedder is deterministic, so the top hit must come from
        // the only document we ingested.
        assert!(
            hits.iter()
                .any(|h| h.text.to_lowercase().contains("cardiology")),
            "top hits should reference the cardiology chunk: {hits:?}"
        );
    }

    #[tokio::test]
    async fn round_trip_ingest_then_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_fixture(tmp.path(), "round.txt", "Material a borrar.");
        let pipeline = fresh_pipeline(&tmp).await;

        let report = pipeline.ingest_path(&file, |_| {}).await.unwrap();
        assert_eq!(report.ingested.len(), 1);
        let id = report.ingested[0].id.clone();

        let listed_before = pipeline.repository.list().unwrap();
        assert_eq!(listed_before.len(), 1);

        let removed = pipeline.repository.remove(&id).await.unwrap();
        assert!(removed);

        let listed_after = pipeline.repository.list().unwrap();
        assert!(listed_after.is_empty());
    }
}
