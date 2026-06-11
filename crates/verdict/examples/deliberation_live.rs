//! Live end-to-end harness: run the FULL four-phase deliberation
//! (briefing → drafting → red-team → finalize) over real case files
//! with the **claude-cli** provider — the exact production path the
//! desktop batch runner uses, minus the `SQLite` persistence.
//!
//! Requires a logged-in `claude` binary on PATH (`claude auth login`).
//! Each file costs ~4 LLM calls against the user's subscription.
//!
//! ```sh
//! cargo run -p conclave-verdict --example deliberation_live -- /path/to/case.pdf [more…]
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use conclave_deident::{Deidentifier, PipelineDeidentifier};
use conclave_providers::ClaudeCliProvider;
use conclave_verdict::deliberation::{
    run_deliberation, DeliberationEvent, DeliberationInputs, DeliberationOptions,
};
use conclave_verdict::persistence::CaseAttachment;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let paths: Vec<std::path::PathBuf> = std::env::args().skip(1).map(Into::into).collect();
    assert!(!paths.is_empty(), "usage: deliberation_live <file> [file…]");

    let provider = Arc::new(ClaudeCliProvider::new());
    let mut failures = 0usize;

    for (n, path) in paths.iter().enumerate() {
        eprintln!(
            "[case {}/{}] {}",
            n + 1,
            paths.len(),
            path.file_name().unwrap_or_default().to_string_lossy()
        );

        // Same ingestion the app performs: extract → de-identify.
        let extracted = conclave_rag::extract_from_path(path).expect("extract");
        let masked = PipelineDeidentifier::new()
            .deidentify(&extracted.content)
            .expect("deidentify");
        eprintln!(
            "[case {}] extracted {} chars, masked {} spans",
            n + 1,
            extracted.content.len(),
            masked.spans.len()
        );

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let attachment = CaseAttachment {
            id: format!("att-live-{n}"),
            case_id: format!("case-live-{n}"),
            position: 1,
            original_filename: filename,
            stored_path: path.display().to_string(),
            sha256: String::new(),
            doc_type: "informe".into(),
            mime: "application/pdf".into(),
            extracted_text: masked.masked_text.clone(),
            needs_ocr: false,
            byte_size: 0,
            created_at: Utc::now(),
        };

        let inputs = DeliberationInputs {
            specialty: "comité oncológico colorrectal".into(),
            output_language: "es".into(),
            rules_block: String::new(),
            masked_case_text: masked.masked_text.clone(),
            user_question: "¿Cuál es el manejo recomendado?".into(),
            active_skill_id: None,
            active_skill_instructions: None,
            evidence_chunks: Vec::new(),
            external_evidence: Vec::new(),
            past_cases: Vec::new(),
            attachments: vec![attachment],
            images: Vec::new(),
        };
        let allowed_refs: HashSet<String> = std::iter::once("A1".to_owned()).collect();

        // Forward phase progress to stderr so a watcher sees liveness.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DeliberationEvent>();
        let case_no = n + 1;
        let printer = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                print_event(case_no, &ev);
            }
        });

        let started = std::time::Instant::now();
        match run_deliberation(
            provider.clone(),
            inputs,
            allowed_refs,
            DeliberationOptions::default(),
            tx,
        )
        .await
        {
            Ok(outcome) => {
                eprintln!(
                    "[case {}] OK in {:?} — model {}, {} in / {} out tokens, recommendation: {}",
                    n + 1,
                    started.elapsed(),
                    outcome.model,
                    outcome.trace.total_input_tokens,
                    outcome.trace.total_output_tokens,
                    truncate(&outcome.verdict.primary_recommendation.action, 160),
                );
            }
            Err(e) => {
                failures += 1;
                eprintln!("[case {}] FAILED in {:?}: {e}", n + 1, started.elapsed());
            }
        }
        let _ = printer.await;
    }

    if failures > 0 {
        eprintln!("RESULT: {failures} case(s) failed");
        std::process::exit(1);
    }
    eprintln!("RESULT: all cases deliberated successfully");
}

fn print_event(case_no: usize, ev: &DeliberationEvent) {
    match ev {
        DeliberationEvent::PhaseStarted { phase } => {
            eprintln!("[case {case_no}] phase {} started", phase.as_str());
        }
        DeliberationEvent::PhaseCompleted { phase, output } => {
            eprintln!(
                "[case {case_no}] phase {} completed ({} chars)",
                phase.as_str(),
                output.len()
            );
        }
        DeliberationEvent::PhaseRetrying {
            phase,
            attempt,
            reason,
        } => {
            eprintln!(
                "[case {case_no}] phase {} retrying (attempt {attempt}): {reason}",
                phase.as_str()
            );
        }
        DeliberationEvent::PhaseFailed { phase, error } => {
            eprintln!("[case {case_no}] phase {} FAILED: {error}", phase.as_str());
        }
        DeliberationEvent::Done { .. } => {}
    }
}

fn truncate(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if end < s.len() {
        format!("{}…", &s[..end])
    } else {
        s.to_owned()
    }
}
