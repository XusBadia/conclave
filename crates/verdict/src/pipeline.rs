//! Top-level verdict orchestration.
//!
//! ```text
//! case text + question
//!         │
//!         ▼
//!    deident (Phase 3) ── original + masked text + spans
//!         │
//!         ▼
//!    retrieval: top-K chunks via rag (Phase 1) ── [E1..EN]
//!         │
//!         ▼
//!    prompt assembly (verdict_v1)
//!         │
//!         ▼
//!    LLM call via configured provider (Phase 2)
//!         │
//!         ▼
//!    schema + citation validation (one retry on either)
//!         │
//!         ▼
//!    persistence (case + verdict + retrieval trace)
//! ```

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use conclave_core::{Error, Result, Workspace, MEDICAL_DISCLAIMER};
use conclave_deident::Deidentifier;
use conclave_providers::{CompletionRequest, LlmProvider, Message, ProviderError};
use conclave_rag::{DocumentRepository, Embedder};

use crate::persistence::{
    CaseAttachment, CaseRecord, CaseStatus, CaseStore, PastCaseHit, RetrievalTrace, VerdictRecord,
};
use crate::prompt::{
    CaseAttachmentInput, EvidenceChunkInput, PastCaseInput, PromptInputs, PromptTemplate,
    VERDICT_PROMPT_VERSION,
};
use crate::schema::Verdict;
use crate::validation::validate_verdict;

/// Configuration knobs for a verdict run.
#[derive(Debug, Clone)]
pub struct VerdictOptions {
    /// Top-K chunks to retrieve from the knowledge base.
    pub top_k: usize,
    /// Workspace rules, formatted as bullet list (or empty).
    pub rules_block: String,
    /// Output language hint passed to the LLM (default `es`).
    pub output_language: String,
    /// Temperature for the LLM call.
    pub temperature: f32,
    /// Max output tokens cap.
    pub max_output_tokens: u32,
    /// How many past cases to inject. 0 disables Phase 5 memory.
    pub past_cases_k: usize,
    /// Minimum cosine similarity for a past case to be considered.
    pub past_cases_min_similarity: f32,
}

impl Default for VerdictOptions {
    fn default() -> Self {
        Self {
            top_k: 8,
            rules_block: String::new(),
            output_language: "es".into(),
            temperature: 0.2,
            max_output_tokens: 2048,
            past_cases_k: 3,
            past_cases_min_similarity: 0.65,
        }
    }
}

/// End-to-end result of one `run` invocation.
#[derive(Debug, Clone)]
pub struct VerdictRun {
    pub case: CaseRecord,
    pub verdict_record: VerdictRecord,
    pub verdict: Verdict,
    pub trace: RetrievalTrace,
}

/// Composes the per-workspace components and runs cases through them.
pub struct VerdictPipeline {
    deident: Box<dyn Deidentifier + Send + Sync>,
    embedder: Arc<dyn Embedder>,
    repository: Arc<DocumentRepository>,
    provider: Arc<dyn LlmProvider>,
    store: Arc<Mutex<CaseStore>>,
    workspace: Workspace,
}

impl std::fmt::Debug for VerdictPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerdictPipeline")
            .field("workspace", &self.workspace.id)
            .field("provider", &self.provider.id())
            .field("embedder", &self.embedder.id())
            .finish_non_exhaustive()
    }
}

impl VerdictPipeline {
    /// Build a pipeline. The caller owns lifetime for the heavy components.
    pub fn new(
        workspace: Workspace,
        deident: Box<dyn Deidentifier + Send + Sync>,
        embedder: Arc<dyn Embedder>,
        repository: Arc<DocumentRepository>,
        provider: Arc<dyn LlmProvider>,
        store: Arc<Mutex<CaseStore>>,
    ) -> Self {
        Self {
            deident,
            embedder,
            repository,
            provider,
            store,
            workspace,
        }
    }

    /// Run the full pipeline over `case_text` + optional `question`,
    /// optionally including files attached to this specific case.
    ///
    /// `attachments` are surfaced to the LLM as the `[A1..AN]` block.
    /// They are *not* indexed into the workspace knowledge base — they
    /// only inform this single verdict.
    pub async fn run(
        &self,
        case_text: &str,
        question: &str,
        attachments: &[CaseAttachment],
        options: &VerdictOptions,
    ) -> Result<VerdictRun> {
        // 1) De-identify.
        let deident_result = self.deident.deidentify(case_text)?;
        let masked_text = deident_result.masked_text.clone();
        let deident_pipeline_id = deident_result.pipeline_id.to_owned();

        // 2) Embed the case once — reused for both KB retrieval and past
        //    case retrieval.
        let case_embedding = self.embed_case(&masked_text).await?;

        // 3) Retrieve evidence.
        let chunks = self
            .retrieve_evidence_with_vec(&case_embedding, options.top_k)
            .await?;
        let evidence_refs: Vec<String> = (1..=chunks.len()).map(|i| format!("E{i}")).collect();

        // 4) Retrieve past cases (Phase 5).
        let past_hits = self.retrieve_past_cases(&case_embedding, options)?;
        let past_refs: Vec<String> = (1..=past_hits.len()).map(|i| format!("P{i}")).collect();

        // 4b) Refs for case-scoped attachments.
        let attachment_refs: Vec<String> =
            (1..=attachments.len()).map(|i| format!("A{i}")).collect();

        // 5) Assemble prompt.
        let evidence_inputs: Vec<EvidenceChunkInput<'_>> = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| EvidenceChunkInput {
                index: i + 1,
                document_title: &c.document_title,
                page: "—",
                doc_type: &c.doc_type,
                snippet: &c.snippet,
            })
            .collect();
        let past_cases_inputs: Vec<PastCaseInput<'_>> = past_hits
            .iter()
            .enumerate()
            .map(|(i, h)| PastCaseInput {
                index: i + 1,
                feedback: h.feedback_kind.map_or("none", |k| k.as_db_str()),
                feedback_reason: h.feedback_reason.as_deref().unwrap_or(""),
                case_summary: &h.case_summary,
                previous_verdict_summary: &h.verdict_summary,
                user_modifications: "",
            })
            .collect();
        let attachment_inputs: Vec<CaseAttachmentInput<'_>> = attachments
            .iter()
            .enumerate()
            .map(|(i, a)| CaseAttachmentInput {
                index: i + 1,
                filename: &a.original_filename,
                doc_type: &a.doc_type,
                snippet: &a.extracted_text,
                needs_ocr: a.needs_ocr,
            })
            .collect();
        let specialty = self
            .workspace
            .specialty
            .as_deref()
            .unwrap_or("medicina general");
        let inputs = PromptInputs {
            specialty,
            output_language: &options.output_language,
            rules_block: &options.rules_block,
            evidence_chunks: &evidence_inputs,
            external_evidence: &[],
            past_cases: &past_cases_inputs,
            case_attachments: &attachment_inputs,
            de_identified_case_text: &masked_text,
            user_question: question,
            disclaimer: MEDICAL_DISCLAIMER,
        };
        let prompt = PromptTemplate.render(&inputs);

        // 4) Call the LLM with one retry on schema / citation failure.
        let mut allowed_refs: HashSet<String> = evidence_refs.iter().cloned().collect();
        for r in &attachment_refs {
            allowed_refs.insert(r.clone());
        }
        let start = Instant::now();
        let (mut response, mut model) = self
            .call_llm(&prompt, options)
            .await
            .map_err(|e| Error::Provider(format!("verdict LLM call failed: {e}")))?;
        let elapsed = start.elapsed().as_millis() as u64;

        let (verdict, parsed_response) = match validate_verdict(&response.text, &allowed_refs) {
            Ok(v) => (v, response),
            Err(first) => {
                tracing::warn!(error = %first, "first verdict response invalid — retrying once");
                let retry_prompt = format!(
                    "{prompt}\n\nYour previous response failed validation ({first}). \
Return the JSON object only, citing only the supplied evidence ids."
                );
                let retry = self
                    .call_llm(&retry_prompt, options)
                    .await
                    .map_err(|e| Error::Provider(format!("verdict retry failed: {e}")))?;
                model = retry.1;
                let parsed = validate_verdict(&retry.0.text, &allowed_refs).map_err(|e| {
                    Error::Provider(format!("verdict still invalid after retry: {e}"))
                })?;
                response = retry.0;
                (parsed, response)
            }
        };

        // 5) Force the disclaimer to the canonical string.
        let mut verdict = verdict;
        verdict.disclaimer = MEDICAL_DISCLAIMER.to_owned();

        // 6) Persist.
        let now = Utc::now();
        let case = CaseRecord {
            id: format!("case-{}", Uuid::new_v4()),
            created_at: now,
            case_date: now,
            workspace_id: self.workspace.id.clone(),
            question: question.to_owned(),
            original_text: case_text.to_owned(),
            masked_text,
            deident_pipeline_id,
            status: CaseStatus::Completed,
            patient_label: String::new(),
            latest_error: None,
        };
        let verdict_record = VerdictRecord {
            id: format!("verdict-{}", Uuid::new_v4()),
            case_id: case.id.clone(),
            prompt_version: VERDICT_PROMPT_VERSION.to_owned(),
            provider_id: self.provider.id().to_owned(),
            model,
            latency_ms: elapsed,
            input_tokens: parsed_response.usage.input_tokens,
            output_tokens: parsed_response.usage.output_tokens,
            output_json: serde_json::to_string(&verdict).unwrap_or_else(|_| String::from("{}")),
            created_at: now,
        };
        let trace = RetrievalTrace {
            verdict_id: verdict_record.id.clone(),
            evidence_refs,
            past_cases_refs: past_refs,
            online_evidence_refs: Vec::new(),
            attachment_refs,
        };

        // Case memory (Phase 5): persist the case summary embedding so it
        // can be retrieved as a past case in future runs.
        let verdict_summary = truncate(
            &format!(
                "{} | {}",
                verdict.primary_recommendation.action, verdict.certainty_justification
            ),
            1_200,
        );
        let case_memory_summary = truncate(&verdict.case_summary, 1_200);

        {
            let store = self
                .store
                .lock()
                .map_err(|_| Error::Rag("case store mutex poisoned".into()))?;
            store.insert_case(&case)?;
            store.insert_verdict(&verdict_record)?;
            store.insert_trace(&trace)?;
            store.upsert_case_memory(
                &case.id,
                &case_embedding,
                &case_memory_summary,
                &verdict_summary,
            )?;
        }

        Ok(VerdictRun {
            case,
            verdict_record,
            verdict,
            trace,
        })
    }

    /// Run the pipeline against an **already persisted** case (typically
    /// a draft). The case row stays the same id; we only insert the
    /// verdict + retrieval trace + memory entry and flip the status to
    /// `Completed`. Used by the `run_draft_case` Tauri command.
    ///
    /// The case's `masked_text` and `question` are taken as-is — the
    /// caller is responsible for having applied any clinical context
    /// edits to the draft row before calling this.
    pub async fn run_for_case(
        &self,
        case: &CaseRecord,
        attachments: &[CaseAttachment],
        options: &VerdictOptions,
    ) -> Result<VerdictRun> {
        // 1) Embed once for KB + past-case retrieval.
        let case_embedding = self.embed_case(&case.masked_text).await?;

        // 2) Retrieve evidence.
        let chunks = self
            .retrieve_evidence_with_vec(&case_embedding, options.top_k)
            .await?;
        let evidence_refs: Vec<String> = (1..=chunks.len()).map(|i| format!("E{i}")).collect();

        // 3) Retrieve past cases.
        let past_hits = self.retrieve_past_cases(&case_embedding, options)?;
        let past_refs: Vec<String> = (1..=past_hits.len()).map(|i| format!("P{i}")).collect();

        // 4) Case-scoped attachment refs.
        let attachment_refs: Vec<String> =
            (1..=attachments.len()).map(|i| format!("A{i}")).collect();

        // 5) Assemble prompt.
        let evidence_inputs: Vec<EvidenceChunkInput<'_>> = chunks
            .iter()
            .enumerate()
            .map(|(i, c)| EvidenceChunkInput {
                index: i + 1,
                document_title: &c.document_title,
                page: "—",
                doc_type: &c.doc_type,
                snippet: &c.snippet,
            })
            .collect();
        let past_cases_inputs: Vec<PastCaseInput<'_>> = past_hits
            .iter()
            .enumerate()
            .map(|(i, h)| PastCaseInput {
                index: i + 1,
                feedback: h.feedback_kind.map_or("none", |k| k.as_db_str()),
                feedback_reason: h.feedback_reason.as_deref().unwrap_or(""),
                case_summary: &h.case_summary,
                previous_verdict_summary: &h.verdict_summary,
                user_modifications: "",
            })
            .collect();
        let attachment_inputs: Vec<CaseAttachmentInput<'_>> = attachments
            .iter()
            .enumerate()
            .map(|(i, a)| CaseAttachmentInput {
                index: i + 1,
                filename: &a.original_filename,
                doc_type: &a.doc_type,
                snippet: &a.extracted_text,
                needs_ocr: a.needs_ocr,
            })
            .collect();
        let specialty = self
            .workspace
            .specialty
            .as_deref()
            .unwrap_or("medicina general");
        let inputs = PromptInputs {
            specialty,
            output_language: &options.output_language,
            rules_block: &options.rules_block,
            evidence_chunks: &evidence_inputs,
            external_evidence: &[],
            past_cases: &past_cases_inputs,
            case_attachments: &attachment_inputs,
            de_identified_case_text: &case.masked_text,
            user_question: &case.question,
            disclaimer: MEDICAL_DISCLAIMER,
        };
        let prompt = PromptTemplate.render(&inputs);

        // 6) Call the LLM with one retry on validation failure.
        let mut allowed_refs: HashSet<String> = evidence_refs.iter().cloned().collect();
        for r in &attachment_refs {
            allowed_refs.insert(r.clone());
        }
        let start = Instant::now();
        let (mut response, mut model) = self
            .call_llm(&prompt, options)
            .await
            .map_err(|e| Error::Provider(format!("draft LLM call failed: {e}")))?;
        let elapsed = start.elapsed().as_millis() as u64;

        let (verdict, parsed_response) = match validate_verdict(&response.text, &allowed_refs) {
            Ok(v) => (v, response),
            Err(first) => {
                tracing::warn!(error = %first, "first draft response invalid — retrying once");
                let retry_prompt = format!(
                    "{prompt}\n\nYour previous response failed validation ({first}). \
Return the JSON object only, citing only the supplied evidence ids."
                );
                let retry = self
                    .call_llm(&retry_prompt, options)
                    .await
                    .map_err(|e| Error::Provider(format!("draft retry failed: {e}")))?;
                model = retry.1;
                let parsed = validate_verdict(&retry.0.text, &allowed_refs).map_err(|e| {
                    Error::Provider(format!("draft verdict still invalid after retry: {e}"))
                })?;
                response = retry.0;
                (parsed, response)
            }
        };

        let mut verdict = verdict;
        verdict.disclaimer = MEDICAL_DISCLAIMER.to_owned();

        // 7) Persist the verdict against the existing case_id and flip
        //    status to Completed. The case row itself is NOT re-inserted.
        let now = Utc::now();
        let verdict_record = VerdictRecord {
            id: format!("verdict-{}", Uuid::new_v4()),
            case_id: case.id.clone(),
            prompt_version: VERDICT_PROMPT_VERSION.to_owned(),
            provider_id: self.provider.id().to_owned(),
            model,
            latency_ms: elapsed,
            input_tokens: parsed_response.usage.input_tokens,
            output_tokens: parsed_response.usage.output_tokens,
            output_json: serde_json::to_string(&verdict).unwrap_or_else(|_| String::from("{}")),
            created_at: now,
        };
        let trace = RetrievalTrace {
            verdict_id: verdict_record.id.clone(),
            evidence_refs,
            past_cases_refs: past_refs,
            online_evidence_refs: Vec::new(),
            attachment_refs,
        };
        let verdict_summary = truncate(
            &format!(
                "{} | {}",
                verdict.primary_recommendation.action, verdict.certainty_justification
            ),
            1_200,
        );
        let case_memory_summary = truncate(&verdict.case_summary, 1_200);

        {
            let store = self
                .store
                .lock()
                .map_err(|_| Error::Rag("case store mutex poisoned".into()))?;
            store.insert_verdict(&verdict_record)?;
            store.insert_trace(&trace)?;
            store.upsert_case_memory(
                &case.id,
                &case_embedding,
                &case_memory_summary,
                &verdict_summary,
            )?;
            store.mark_case_status(&case.id, CaseStatus::Completed)?;
        }

        let mut promoted_case = case.clone();
        promoted_case.status = CaseStatus::Completed;
        Ok(VerdictRun {
            case: promoted_case,
            verdict_record,
            verdict,
            trace,
        })
    }

    async fn embed_case(&self, masked_text: &str) -> Result<Vec<f32>> {
        let embedder = Arc::clone(&self.embedder);
        let text = masked_text.to_owned();
        let vectors = tokio::task::spawn_blocking(move || embedder.embed(&[text]))
            .await
            .map_err(|e| Error::Rag(format!("embed task join: {e}")))??;
        vectors
            .into_iter()
            .next()
            .ok_or_else(|| Error::Rag("embedder returned no vectors".into()))
    }

    async fn retrieve_evidence_with_vec(
        &self,
        query_vec: &[f32],
        top_k: usize,
    ) -> Result<Vec<EvidenceChunk>> {
        if top_k == 0 {
            return Ok(Vec::new());
        }
        let hits = self.repository.search(query_vec, top_k).await?;
        let mut out = Vec::with_capacity(hits.len());
        for h in hits {
            let details = self.repository.show(&h.document_id)?;
            let (title, doc_type) = match details {
                Some(d) => (
                    d.record.title,
                    format!("{:?}", d.record.doc_type).to_lowercase(),
                ),
                None => (h.document_id.clone(), "unknown".into()),
            };
            out.push(EvidenceChunk {
                document_title: title,
                doc_type,
                snippet: truncate(&h.text, 1_200),
            });
        }
        Ok(out)
    }

    fn retrieve_past_cases(
        &self,
        case_embedding: &[f32],
        options: &VerdictOptions,
    ) -> Result<Vec<PastCaseHit>> {
        if options.past_cases_k == 0 {
            return Ok(Vec::new());
        }
        let store = self
            .store
            .lock()
            .map_err(|_| Error::Rag("case store mutex poisoned".into()))?;
        store.similar_past_cases(
            case_embedding,
            options.past_cases_k,
            options.past_cases_min_similarity,
        )
    }

    async fn call_llm(
        &self,
        prompt: &str,
        options: &VerdictOptions,
    ) -> std::result::Result<(conclave_providers::CompletionResponse, String), ProviderError> {
        let req = CompletionRequest {
            model: String::new(),
            messages: vec![Message::user(prompt.to_owned())],
            max_output_tokens: Some(options.max_output_tokens),
            temperature: Some(options.temperature),
            json_schema: Some(serde_json::json!({"type": "object"})),
            allow_web_search: false,
            images: Vec::new(),
        };
        let resp = self.provider.complete(req).await?;
        let model = resp.model.clone();
        Ok((resp, model))
    }
}

#[derive(Debug, Clone)]
struct EvidenceChunk {
    document_title: String,
    doc_type: String,
    snippet: String,
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use conclave_deident::PipelineDeidentifier;
    use conclave_providers::MockProvider;
    use conclave_rag::{MockEmbedder, RepositoryLayout};

    fn sample_workspace() -> Workspace {
        Workspace {
            id: "test-ws".into(),
            name: "Test".into(),
            specialty: Some("cardiología".into()),
            language: Some("es".into()),
            created_at: Utc::now(),
        }
    }

    fn sample_verdict_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "case_summary": "Paciente con disnea súbita.",
            "key_clinical_data": [
                {"label": "TA", "value": "150/90 mmHg"}
            ],
            "applied_evidence": [],
            "primary_recommendation": {
                "action": "Estabilizar y solicitar ECG y troponinas.",
                "rationale": "Síntomas compatibles con SCA."
            },
            "alternatives": [],
            "certainty_level": "medium",
            "certainty_justification": "Datos clínicos limitados.",
            "red_flags": ["TA elevada"],
            "follow_up_triggers": ["Repetir troponinas a las 3h"],
            "disclaimer": "stub"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn happy_path_end_to_end_with_mocks() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RepositoryLayout::new(tmp.path().join("ws"));
        let repo = Arc::new(
            DocumentRepository::open(layout, MockEmbedder::new().dim())
                .await
                .unwrap(),
        );
        let store = Arc::new(Mutex::new(
            CaseStore::open(tmp.path().join("cases.sqlite")).unwrap(),
        ));
        let provider: Arc<dyn LlmProvider> =
            Arc::new(MockProvider::with_response(sample_verdict_json()));
        let pipeline = VerdictPipeline::new(
            sample_workspace(),
            Box::new(PipelineDeidentifier::new()),
            Arc::new(MockEmbedder::new()),
            repo,
            provider,
            store,
        );

        let run = pipeline
            .run(
                "Paciente de 60 años con disnea.",
                "Manejo inicial?",
                &[],
                &VerdictOptions::default(),
            )
            .await
            .unwrap();

        assert_eq!(run.verdict.primary_recommendation.action.is_empty(), false);
        assert_eq!(run.verdict.certainty_level, crate::CertaintyLevel::Medium);
        // Disclaimer is forced to the canonical text.
        assert!(run
            .verdict
            .disclaimer
            .starts_with("Conclave is an experimental"));
        assert_eq!(run.case.workspace_id, "test-ws");
        assert_eq!(run.verdict_record.prompt_version, VERDICT_PROMPT_VERSION);
    }
}
