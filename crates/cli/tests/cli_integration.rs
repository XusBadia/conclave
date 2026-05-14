//! End-to-end integration tests for `conclave-cli`.
//!
//! These drive the compiled binary in a sandboxed `--workspace-root`,
//! using the deterministic `mock` embedder so they stay fast and offline.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    // Cargo sets CARGO_BIN_EXE_<name> for every bin in the same package.
    let raw = env!("CARGO_BIN_EXE_conclave-cli");
    PathBuf::from(raw)
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new(binary())
        .arg("--workspace-root")
        .arg(root)
        .arg("--no-disclaimer")
        .args(args)
        .output()
        .expect("spawn conclave-cli");
    assert!(
        out.status.success(),
        "conclave-cli {args:?} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}",
        stdout = String::from_utf8_lossy(&out.stdout),
        stderr = String::from_utf8_lossy(&out.stderr),
    );
    out
}

fn prepare_workspace(root: &Path) {
    let cfg = root.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("conclave.toml"),
        r#"[general]
default_workspace = "default"
log_format = "pretty"

[rag]
chunk_size = 256
chunk_overlap = 32
top_k = 5

[knowledge]
embedding_model = "mock"
embedding_dim = 96
bm25_weight = 1.0
dense_weight = 1.0
rrf_k = 60.0

[providers]
"#,
    )
    .unwrap();
}

fn seed_corpus(corpus: &Path) {
    std::fs::create_dir_all(corpus).unwrap();
    std::fs::write(
        corpus.join("iamcest.md"),
        "# Manejo del IAMCEST\n\n\
         Reperfusión primaria con angioplastia antes de 120 minutos.\n\
         Antiagregación con AAS y prasugrel.\n",
    )
    .unwrap();
    std::fs::write(
        corpus.join("ictus.md"),
        "# Ictus isquémico agudo\n\n\
         Trombólisis intravenosa con alteplasa en ventana <4.5 h.\n\
         Trombectomía mecánica para oclusión de gran vaso.\n",
    )
    .unwrap();
    std::fs::write(
        corpus.join("sepsis.txt"),
        "Sepsis y shock séptico\n\
         Antibiótico empírico en la primera hora.\n\
         Cristaloides 30 ml/kg.\n\
         Vasopresor noradrenalina.\n",
    )
    .unwrap();
}

#[test]
fn ingest_search_stats_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("conclave");
    let corpus = tmp.path().join("corpus");
    prepare_workspace(&root);
    seed_corpus(&corpus);

    let out = run(&root, &["ingest", corpus.to_str().unwrap()]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("inserted=3"), "stdout was:\n{stdout}");

    let stats = run(&root, &["workspace", "stats"]);
    let stats_stdout = String::from_utf8(stats.stdout).unwrap();
    assert!(
        stats_stdout.contains("documents:  3"),
        "stats:\n{stats_stdout}"
    );
    assert!(stats_stdout.contains("chunks:"), "stats:\n{stats_stdout}");

    let search = run(
        &root,
        &[
            "search",
            "reperfusión angioplastia primaria",
            "--top-k",
            "3",
        ],
    );
    let search_stdout = String::from_utf8(search.stdout).unwrap();
    assert!(
        search_stdout.contains("Manejo del IAMCEST"),
        "search did not return cardio doc:\n{search_stdout}"
    );

    // Re-ingest should be idempotent.
    let again = run(&root, &["ingest", corpus.to_str().unwrap()]);
    let again_stdout = String::from_utf8(again.stdout).unwrap();
    assert!(
        again_stdout.contains("unchanged=3"),
        "re-ingest was not idempotent:\n{again_stdout}"
    );
}

#[test]
fn search_without_kb_errors_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("conclave");
    prepare_workspace(&root);

    let out = Command::new(binary())
        .arg("--workspace-root")
        .arg(&root)
        .arg("--no-disclaimer")
        .args(["search", "anything"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("no knowledge base"),
        "expected hint, got:\n{stderr}"
    );
}
