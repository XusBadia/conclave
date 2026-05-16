//! End-to-end ingestion + search integration test, exercising the public
//! `conclave_rag` surface with on-disk fixtures and the deterministic mock
//! embedder. PDF/DOCX/OCR are out of scope — those need binary fixtures
//! generated outside the build and are exercised separately.

use std::path::PathBuf;
use std::sync::Arc;

use conclave_rag::{
    ChunkParams, DocumentRepository, Embedder, IngestionPipeline, MockEmbedder, RepositoryLayout,
};

fn fixtures_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests").join("fixtures")
}

async fn fresh_pipeline(tmp: &tempfile::TempDir) -> IngestionPipeline {
    let layout = RepositoryLayout::new(tmp.path().join("workspace"));
    let dim = MockEmbedder::new().dim();
    let repo = Arc::new(DocumentRepository::open(layout, dim).await.unwrap());
    IngestionPipeline::new(
        Arc::new(MockEmbedder::new()),
        repo,
        ChunkParams::DEFAULT,
    )
    .unwrap()
}

#[tokio::test]
async fn ingest_three_fixtures_then_search_each() {
    let tmp = tempfile::tempdir().unwrap();
    let pipeline = fresh_pipeline(&tmp).await;

    let report = pipeline
        .ingest_path(&fixtures_dir(), |_| {})
        .await
        .unwrap();
    assert!(report.failed.is_empty(), "no doc should fail: {:?}", report.failed);
    assert_eq!(
        report.ingested.len(),
        3,
        "expected txt + md + html, got {} ingested",
        report.ingested.len(),
    );

    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new());

    for query in ["furosemida", "metformina", "tiazidicos"] {
        let vec = embedder.embed(&[query.to_string()]).unwrap();
        let hits = pipeline
            .repository()
            .search(&vec[0], 5)
            .await
            .unwrap();
        assert!(
            hits.iter().any(|h| h.text.to_lowercase().contains(query)),
            "term `{query}` should surface in the top-5: {hits:?}",
        );
    }
}

#[tokio::test]
async fn round_trip_ingest_then_remove_each_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let pipeline = fresh_pipeline(&tmp).await;

    let report = pipeline
        .ingest_path(&fixtures_dir(), |_| {})
        .await
        .unwrap();
    assert_eq!(report.ingested.len(), 3);

    for doc in &report.ingested {
        let removed = pipeline.repository().remove(&doc.id).await.unwrap();
        assert!(removed, "expected to delete {}", doc.id);
    }

    let listed = pipeline.repository().list().unwrap();
    assert!(listed.is_empty(), "every document should be gone, got {listed:?}");
}
