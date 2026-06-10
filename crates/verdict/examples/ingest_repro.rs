//! Diagnostic harness: run the FULL case-attachment ingestion pipeline
//! (copy → extract → de-identify) on one or more files, with per-stage
//! timing. Reproduces batch wedges on problem attachments:
//!
//! ```sh
//! cargo run -p conclave-verdict --example ingest_repro -- /path/to/file.pdf
//! ```

use conclave_deident::{Deidentifier, PipelineDeidentifier};

fn main() {
    let paths: Vec<std::path::PathBuf> = std::env::args().skip(1).map(Into::into).collect();
    assert!(!paths.is_empty(), "usage: ingest_repro <file> [file…]");

    // Stage timings first, outside the full pipeline, so a wedge tells
    // us WHICH stage is stuck.
    for p in &paths {
        let t0 = std::time::Instant::now();
        let extracted = conclave_rag::extract_from_path(p).expect("extract");
        eprintln!(
            "[stage] extract {}: {} chars in {:?}",
            p.display(),
            extracted.content.len(),
            t0.elapsed()
        );
        let deid = PipelineDeidentifier::new();
        let t1 = std::time::Instant::now();
        let masked = deid.deidentify(&extracted.content).expect("deidentify");
        eprintln!(
            "[stage] deidentify {}: {} spans, {} chars in {:?}",
            p.display(),
            masked.spans.len(),
            masked.masked_text.len(),
            t1.elapsed()
        );
    }

    // Now the real production entry point, end to end.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let tmp = tempfile::tempdir().expect("tempdir");
    let deid = PipelineDeidentifier::new();
    let t2 = std::time::Instant::now();
    let out = rt
        .block_on(conclave_verdict::ingest_case_attachments(
            paths,
            "case-repro",
            tmp.path(),
            &deid,
        ))
        .expect("ingest_case_attachments");
    println!(
        "ok: {} attachments ingested in {:?}",
        out.len(),
        t2.elapsed()
    );
}
