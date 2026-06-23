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
use conclave_evidence::EvidenceItem;
use conclave_providers::{CompletionRequest, LlmProvider, Message, ProviderError};
use conclave_rag::{DocumentRepository, Embedder};

use crate::persistence::{
    AuditRunRecord, CaseAttachment, CaseRecord, CaseStatus, CaseStore, PastCaseHit, RetrievalTrace,
    VerdictRecord,
};
use crate::prompt::{
    CaseAttachmentInput, EvidenceChunkInput, ExternalEvidenceInput, PastCaseInput, PromptInputs,
    PromptTemplate, VERDICT_PROMPT_VERSION,
};
use crate::schema::Verdict;
use crate::validation::validate_verdict;
use crate::{sha256_hex, AuditPayloadMode, DataBoundaryMode, RawTextRetention};

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
    /// Runtime data-boundary mode.
    pub data_boundary_mode: DataBoundaryMode,
    /// Audit payload posture. Default is fingerprint-only.
    pub audit_payload_mode: AuditPayloadMode,
    /// Optional skill id persisted in audit metadata.
    pub active_skill_id: Option<String>,
    /// Optional markdown skill body appended to the prompt rules.
    pub active_skill_instructions: Option<String>,
    /// Keep raw text locally after a successful run.
    pub retain_raw_text: bool,
    /// When raw text is NOT retained, also delete the original attachment
    /// files from disk after the run (and zero their stored paths). Off by
    /// default: attachments stay viewable in the case detail UI and the
    /// audit row records them as retained.
    pub purge_attachment_files: bool,
    /// Optional live literature supplied by an opt-in external evidence
    /// lookup. These become `X1..Xn` refs in the verdict prompt.
    pub external_evidence: Vec<EvidenceItem>,
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
            data_boundary_mode: DataBoundaryMode::default(),
            audit_payload_mode: AuditPayloadMode::default(),
            active_skill_id: None,
            active_skill_instructions: None,
            retain_raw_text: false,
            purge_attachment_files: false,
            external_evidence: Vec::new(),
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
        self.ensure_runtime_boundary(attachments, options)?;

        // 1) De-identify.
        let deident_result = self.deident.deidentify(case_text)?;
        let masked_text = deident_result.masked_text.clone();
        let deident_pipeline_id = deident_result.pipeline_id.to_owned();
        let raw_text_sha256 = if case_text.is_empty() {
            String::new()
        } else {
            sha256_hex(case_text.as_bytes())
        };

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
        let external_rows = external_prompt_rows(&options.external_evidence);
        let online_refs: Vec<String> = (1..=external_rows.len()).map(|i| format!("X{i}")).collect();

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
        let external_inputs: Vec<ExternalEvidenceInput<'_>> = external_rows
            .iter()
            .enumerate()
            .map(|(i, e)| ExternalEvidenceInput {
                index: i + 1,
                source: &e.source,
                title: &e.title,
                authors: &e.authors,
                year: &e.year,
                venue: &e.venue,
                url: &e.url,
                abstract_text: &e.abstract_text,
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
            active_skill_id: options.active_skill_id.as_deref(),
            active_skill_instructions: options.active_skill_instructions.as_deref(),
            evidence_chunks: &evidence_inputs,
            external_evidence: &external_inputs,
            past_cases: &past_cases_inputs,
            case_attachments: &attachment_inputs,
            de_identified_case_text: &masked_text,
            user_question: question,
            disclaimer: MEDICAL_DISCLAIMER,
        };
        let prompt = PromptTemplate.render(&inputs);

        // 4) Call the LLM with one retry on schema / citation failure.
        let mut allowed_refs: HashSet<String> = evidence_refs.iter().cloned().collect();
        for r in &past_refs {
            allowed_refs.insert(r.clone());
        }
        for r in &attachment_refs {
            allowed_refs.insert(r.clone());
        }
        for r in &online_refs {
            allowed_refs.insert(r.clone());
        }
        let prompt_hash = sha256_hex(prompt.as_bytes());
        let audit_started_at = Utc::now();
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
        let raw_retention = if options.retain_raw_text {
            RawTextRetention::ExplicitRetained
        } else {
            RawTextRetention::Discarded
        };
        let output_json = serde_json::to_string(&verdict).unwrap_or_else(|_| String::from("{}"));
        let case = CaseRecord {
            id: format!("case-{}", Uuid::new_v4()),
            created_at: now,
            case_date: now,
            workspace_id: self.workspace.id.clone(),
            question: question.to_owned(),
            original_text: if options.retain_raw_text {
                case_text.to_owned()
            } else {
                String::new()
            },
            masked_text,
            deident_pipeline_id,
            status: CaseStatus::ReviewReady,
            patient_label: String::new(),
            latest_error: None,
            raw_text_sha256,
            raw_text_retention: raw_retention,
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
            output_json: output_json.clone(),
            created_at: now,
        };
        let trace = RetrievalTrace {
            verdict_id: verdict_record.id.clone(),
            evidence_refs,
            past_cases_refs: past_refs,
            online_evidence_refs: online_refs,
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
            store.insert_audit_run(&AuditRunRecord {
                id: format!("audit-{}", Uuid::new_v4()),
                case_id: case.id.clone(),
                verdict_id: Some(verdict_record.id.clone()),
                provider_id: self.provider.id().to_owned(),
                model: verdict_record.model.clone(),
                data_boundary_mode: options.data_boundary_mode,
                payload_mode: options.audit_payload_mode,
                active_skill_id: options.active_skill_id.clone(),
                started_at: audit_started_at,
                completed_at: Some(now),
                latency_ms: elapsed,
                input_tokens: parsed_response.usage.input_tokens,
                output_tokens: parsed_response.usage.output_tokens,
                prompt_sha256: prompt_hash,
                output_sha256: sha256_hex(output_json.as_bytes()),
                evidence_refs: trace.evidence_refs.clone(),
                past_cases_refs: trace.past_cases_refs.clone(),
                online_evidence_refs: trace.online_evidence_refs.clone(),
                attachment_refs: trace.attachment_refs.clone(),
                raw_text_retention: raw_retention,
                // Retained unless this run's policy actually purges files:
                // must stay the exact negation of the purge condition below.
                attachments_retained: options.retain_raw_text || !options.purge_attachment_files,
                status: "success".into(),
                error: None,
            })?;
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
        self.ensure_runtime_boundary(attachments, options)?;

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
        let external_rows = external_prompt_rows(&options.external_evidence);
        let online_refs: Vec<String> = (1..=external_rows.len()).map(|i| format!("X{i}")).collect();

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
        let external_inputs: Vec<ExternalEvidenceInput<'_>> = external_rows
            .iter()
            .enumerate()
            .map(|(i, e)| ExternalEvidenceInput {
                index: i + 1,
                source: &e.source,
                title: &e.title,
                authors: &e.authors,
                year: &e.year,
                venue: &e.venue,
                url: &e.url,
                abstract_text: &e.abstract_text,
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
            active_skill_id: options.active_skill_id.as_deref(),
            active_skill_instructions: options.active_skill_instructions.as_deref(),
            evidence_chunks: &evidence_inputs,
            external_evidence: &external_inputs,
            past_cases: &past_cases_inputs,
            case_attachments: &attachment_inputs,
            de_identified_case_text: &case.masked_text,
            user_question: &case.question,
            disclaimer: MEDICAL_DISCLAIMER,
        };
        let prompt = PromptTemplate.render(&inputs);

        // 6) Call the LLM with one retry on validation failure.
        let mut allowed_refs: HashSet<String> = evidence_refs.iter().cloned().collect();
        for r in &past_refs {
            allowed_refs.insert(r.clone());
        }
        for r in &attachment_refs {
            allowed_refs.insert(r.clone());
        }
        for r in &online_refs {
            allowed_refs.insert(r.clone());
        }
        let prompt_hash = sha256_hex(prompt.as_bytes());
        let audit_started_at = Utc::now();
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
        //    status to ReviewReady. The case row itself is NOT re-inserted.
        let now = Utc::now();
        let output_json = serde_json::to_string(&verdict).unwrap_or_else(|_| String::from("{}"));
        let verdict_record = VerdictRecord {
            id: format!("verdict-{}", Uuid::new_v4()),
            case_id: case.id.clone(),
            prompt_version: VERDICT_PROMPT_VERSION.to_owned(),
            provider_id: self.provider.id().to_owned(),
            model,
            latency_ms: elapsed,
            input_tokens: parsed_response.usage.input_tokens,
            output_tokens: parsed_response.usage.output_tokens,
            output_json: output_json.clone(),
            created_at: now,
        };
        let trace = RetrievalTrace {
            verdict_id: verdict_record.id.clone(),
            evidence_refs,
            past_cases_refs: past_refs,
            online_evidence_refs: online_refs,
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
            let raw_retention = if options.retain_raw_text {
                case.raw_text_retention
            } else {
                RawTextRetention::Discarded
            };
            store.insert_audit_run(&AuditRunRecord {
                id: format!("audit-{}", Uuid::new_v4()),
                case_id: case.id.clone(),
                verdict_id: Some(verdict_record.id.clone()),
                provider_id: self.provider.id().to_owned(),
                model: verdict_record.model.clone(),
                data_boundary_mode: options.data_boundary_mode,
                payload_mode: options.audit_payload_mode,
                active_skill_id: options.active_skill_id.clone(),
                started_at: audit_started_at,
                completed_at: Some(now),
                latency_ms: elapsed,
                input_tokens: parsed_response.usage.input_tokens,
                output_tokens: parsed_response.usage.output_tokens,
                prompt_sha256: prompt_hash,
                output_sha256: sha256_hex(output_json.as_bytes()),
                evidence_refs: trace.evidence_refs.clone(),
                past_cases_refs: trace.past_cases_refs.clone(),
                online_evidence_refs: trace.online_evidence_refs.clone(),
                attachment_refs: trace.attachment_refs.clone(),
                raw_text_retention: raw_retention,
                // Retained unless this run's policy actually purges files:
                // must stay the exact negation of the purge condition below.
                attachments_retained: options.retain_raw_text || !options.purge_attachment_files,
                status: "success".into(),
                error: None,
            })?;
            store.upsert_case_memory(
                &case.id,
                &case_embedding,
                &case_memory_summary,
                &verdict_summary,
            )?;
            store.mark_case_status(&case.id, CaseStatus::ReviewReady)?;
            if !options.retain_raw_text {
                store.purge_case_phi(&case.id)?;
                if options.purge_attachment_files {
                    store.purge_case_attachment_files(&case.id)?;
                }
            }
        }

        let mut promoted_case = case.clone();
        promoted_case.status = CaseStatus::ReviewReady;
        if !options.retain_raw_text {
            promoted_case.original_text.clear();
            promoted_case.raw_text_retention = RawTextRetention::Discarded;
        }
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

    fn ensure_runtime_boundary(
        &self,
        attachments: &[CaseAttachment],
        options: &VerdictOptions,
    ) -> Result<()> {
        if matches!(options.data_boundary_mode, DataBoundaryMode::LocalOnly)
            && self.provider.requires_network()
        {
            return Err(Error::invalid_config(format!(
                "local_only blocks network provider `{}`",
                self.provider.id()
            )));
        }
        if matches!(options.data_boundary_mode, DataBoundaryMode::LocalOnly)
            && !options.external_evidence.is_empty()
        {
            return Err(Error::invalid_config(
                "local_only blocks online evidence lookup",
            ));
        }
        let has_image_attachment = attachments
            .iter()
            .any(|a| a.doc_type.eq_ignore_ascii_case("image"));
        if has_image_attachment
            && self.provider.requires_network()
            && !matches!(options.data_boundary_mode, DataBoundaryMode::ExplicitPhi)
        {
            return Err(Error::invalid_config(
                "cloud vision requires explicit_phi mode",
            ));
        }
        Ok(())
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
            // Real Verdict schema so schema-enforcing providers
            // (claude-cli) reject malformed shapes at generation time.
            json_schema: Some(crate::schema::verdict_json_schema()),
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

#[derive(Debug, Clone)]
struct ExternalEvidencePromptRow {
    source: String,
    title: String,
    authors: String,
    year: String,
    venue: String,
    url: String,
    abstract_text: String,
}

fn external_prompt_rows(items: &[EvidenceItem]) -> Vec<ExternalEvidencePromptRow> {
    items
        .iter()
        .map(|item| ExternalEvidencePromptRow {
            source: item.source.clone(),
            title: item.title.clone(),
            authors: authors_label(&item.authors),
            year: item.year.map_or_else(|| "?".to_owned(), |y| y.to_string()),
            venue: item.venue.clone().unwrap_or_else(|| "?".to_owned()),
            url: item.url.clone(),
            abstract_text: item
                .abstract_text
                .clone()
                .unwrap_or_else(|| "No abstract available from the external source.".to_owned()),
        })
        .collect()
}

fn authors_label(authors: &[String]) -> String {
    match authors {
        [] => "?".to_owned(),
        [one] => one.clone(),
        [one, two] => format!("{one}, {two}"),
        [one, ..] => format!("{one} et al."),
    }
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
    use async_trait::async_trait;
    use chrono::Utc;
    use conclave_deident::PipelineDeidentifier;
    use conclave_providers::{
        CompletionResponse, MockProvider, ProviderCapabilities, ProviderScope, Usage,
    };
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
            },            "certainty_level": "medium",
            "certainty_justification": "Datos clínicos limitados.",
            "red_flags": ["TA elevada"],
            "follow_up_triggers": ["Repetir troponinas a las 3h"],
            "disclaimer": "stub"
        }))
        .unwrap()
    }

    fn sample_verdict_json_with_x_ref() -> String {
        serde_json::to_string(&serde_json::json!({
            "case_summary": "Paciente con disnea súbita.",
            "key_clinical_data": [],
            "applied_evidence": [{"ref": "X1", "claim": "La literatura externa apoya estratificación inicial."}],
            "primary_recommendation": {
                "action": "Estratificar riesgo y validar con protocolo local.",
                "rationale": "La evidencia externa es auxiliar y no validada por el centro."
            },            "certainty_level": "low",
            "certainty_justification": "Evidencia externa sin protocolo local suficiente.",
            "red_flags": [],
            "follow_up_triggers": [],
            "disclaimer": "stub"
        }))
        .unwrap()
    }

    #[derive(Debug)]
    struct NetworkProvider;

    #[async_trait]
    impl LlmProvider for NetworkProvider {
        fn id(&self) -> &'static str {
            "network-test"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                max_context_tokens: 200_000,
                supports_json_mode: true,
                supports_streaming: false,
                vision: false,
                scope: ProviderScope::General,
            }
        }

        fn requires_network(&self) -> bool {
            true
        }

        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> std::result::Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                text: sample_verdict_json(),
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                model: "network-test".into(),
                web_citations: Vec::new(),
            })
        }
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

    /// Persist a draft with one real attachment file on disk, run it with
    /// `retain_raw_text: false`, and return the bits the assertions need.
    async fn run_draft_with_attachment(
        purge_attachment_files: bool,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<Mutex<CaseStore>>,
        String,
    ) {
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
        let att_body = "informe clínico adjunto";
        let att_path = tmp.path().join("informe.txt");
        std::fs::write(&att_path, att_body).unwrap();
        let case = CaseRecord {
            id: "case-att".into(),
            created_at: Utc::now(),
            case_date: Utc::now(),
            workspace_id: "test-ws".into(),
            question: "Manejo?".into(),
            original_text: "Paciente con disnea.".into(),
            masked_text: "Paciente con disnea.".into(),
            deident_pipeline_id: "test".into(),
            status: CaseStatus::Draft,
            patient_label: String::new(),
            latest_error: None,
            raw_text_sha256: String::new(),
            raw_text_retention: RawTextRetention::TemporaryDraft,
        };
        let attachment = CaseAttachment {
            id: "att-1".into(),
            case_id: case.id.clone(),
            position: 1,
            original_filename: "informe.txt".into(),
            stored_path: att_path.to_string_lossy().into_owned(),
            sha256: String::new(),
            doc_type: "informe".into(),
            mime: "text/plain".into(),
            extracted_text: att_body.into(),
            needs_ocr: false,
            byte_size: att_body.len() as u64,
            created_at: Utc::now(),
        };
        {
            let g = store.lock().unwrap();
            g.insert_case(&case).unwrap();
            g.insert_attachment(&attachment).unwrap();
        }
        let provider: Arc<dyn LlmProvider> =
            Arc::new(MockProvider::with_response(sample_verdict_json()));
        let pipeline = VerdictPipeline::new(
            sample_workspace(),
            Box::new(PipelineDeidentifier::new()),
            Arc::new(MockEmbedder::new()),
            repo,
            provider,
            Arc::clone(&store),
        );
        let options = VerdictOptions {
            retain_raw_text: false,
            purge_attachment_files,
            ..Default::default()
        };
        let run = pipeline
            .run_for_case(&case, &[attachment], &options)
            .await
            .unwrap();
        (tmp, att_path, store, run.case.id)
    }

    #[tokio::test]
    async fn purge_option_deletes_attachment_files_and_audits_it() {
        let (_tmp, att_path, store, case_id) = run_draft_with_attachment(true).await;
        assert!(
            !att_path.exists(),
            "attachment file must be deleted when purge_attachment_files is on"
        );
        let g = store.lock().unwrap();
        let atts = g.list_attachments_for_case(&case_id).unwrap();
        assert_eq!(atts[0].stored_path, "");
        let audit = g.latest_audit_for_case(&case_id).unwrap().unwrap();
        assert!(!audit.attachments_retained);
    }

    #[tokio::test]
    async fn default_keeps_attachment_files_and_audits_retention() {
        let (_tmp, att_path, store, case_id) = run_draft_with_attachment(false).await;
        assert!(
            att_path.exists(),
            "attachment files must survive by default"
        );
        let audit = store
            .lock()
            .unwrap()
            .latest_audit_for_case(&case_id)
            .unwrap()
            .unwrap();
        assert!(audit.attachments_retained);
    }

    #[tokio::test]
    async fn external_evidence_creates_x_refs() {
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
            Arc::new(MockProvider::with_response(sample_verdict_json_with_x_ref()));
        let pipeline = VerdictPipeline::new(
            sample_workspace(),
            Box::new(PipelineDeidentifier::new()),
            Arc::new(MockEmbedder::new()),
            repo,
            provider,
            Arc::clone(&store),
        );
        let options = VerdictOptions {
            external_evidence: vec![EvidenceItem {
                source: "pubmed".into(),
                id: "1".into(),
                title: "External paper".into(),
                authors: vec!["Smith".into()],
                year: Some(2025),
                venue: Some("Journal".into()),
                abstract_text: Some("Abstract".into()),
                url: "https://pubmed.ncbi.nlm.nih.gov/1/".into(),
            }],
            ..Default::default()
        };

        let run = pipeline
            .run("Paciente con disnea.", "Manejo?", &[], &options)
            .await
            .unwrap();

        assert_eq!(run.trace.online_evidence_refs, vec!["X1"]);
        assert_eq!(run.verdict.applied_evidence[0].reference, "X1");
        let audit = store
            .lock()
            .unwrap()
            .latest_audit_for_case(&run.case.id)
            .unwrap()
            .unwrap();
        assert_eq!(audit.online_evidence_refs, vec!["X1"]);
    }

    /// Golden cases (promised by ARCHITECTURE.md §Testing): fixture-driven
    /// end-to-end runs over the mock provider. They pin (a) that the
    /// pipeline survives realistic clinical text — including the
    /// multibyte/emoji class that once panicked deident — and (b) the
    /// de-identification contract: no fixture PII literal may survive into
    /// the masked text or the persisted case row.
    mod golden {
        use super::*;

        #[derive(serde::Deserialize)]
        struct GoldenCase {
            name: String,
            text: String,
            question: String,
            pii: Vec<String>,
        }

        struct GoldenRun {
            fixture: GoldenCase,
            run: VerdictRun,
            store: Arc<Mutex<CaseStore>>,
            _tmp: tempfile::TempDir,
        }

        async fn run_fixture(raw: &str, response: String, options: VerdictOptions) -> GoldenRun {
            let fixture: GoldenCase = serde_json::from_str(raw).unwrap();
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
            let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::with_response(response));
            let pipeline = VerdictPipeline::new(
                sample_workspace(),
                Box::new(PipelineDeidentifier::new()),
                Arc::new(MockEmbedder::new()),
                repo,
                provider,
                Arc::clone(&store),
            );
            let run = pipeline
                .run(&fixture.text, &fixture.question, &[], &options)
                .await
                .unwrap_or_else(|e| panic!("{}: pipeline failed: {e}", fixture.name));
            GoldenRun {
                fixture,
                run,
                store,
                _tmp: tmp,
            }
        }

        #[tokio::test]
        async fn golden_cases_mask_pii_end_to_end() {
            for raw in [
                include_str!("../tests/fixtures/golden_case_es.json"),
                include_str!("../tests/fixtures/golden_case_multibyte.json"),
            ] {
                let GoldenRun {
                    fixture,
                    run,
                    store,
                    _tmp,
                } = run_fixture(raw, sample_verdict_json(), VerdictOptions::default()).await;

                for pii in &fixture.pii {
                    assert!(
                        !run.case.masked_text.contains(pii.as_str()),
                        "{}: PII `{pii}` survived into masked_text: {}",
                        fixture.name,
                        run.case.masked_text
                    );
                }
                // Default options discard raw text: the in-memory case must
                // not carry the original narrative either.
                assert!(run.case.original_text.is_empty(), "{}", fixture.name);

                // Structure stability of the verdict surface.
                assert!(!run.verdict.case_summary.is_empty());
                assert!(!run.verdict.primary_recommendation.action.is_empty());
                assert!(run
                    .verdict
                    .disclaimer
                    .starts_with("Conclave is an experimental"));

                // The persisted row and the audit trail match the contract.
                let g = store.lock().unwrap();
                let persisted = g.get_case(&run.case.id).unwrap().unwrap();
                assert!(persisted.original_text.is_empty(), "{}", fixture.name);
                for pii in &fixture.pii {
                    assert!(
                        !persisted.masked_text.contains(pii.as_str()),
                        "{}: PII `{pii}` persisted to SQLite",
                        fixture.name
                    );
                }
                let audit = g.latest_audit_for_case(&run.case.id).unwrap().unwrap();
                assert_eq!(audit.status, "success");
                assert!(audit.attachments_retained);
            }
        }

        #[tokio::test]
        async fn golden_evidence_case_wires_x_refs() {
            let options = VerdictOptions {
                external_evidence: vec![EvidenceItem {
                    source: "pubmed".into(),
                    id: "1".into(),
                    title: "External paper".into(),
                    authors: vec!["Smith".into()],
                    year: Some(2026),
                    venue: Some("Journal".into()),
                    abstract_text: Some("Abstract".into()),
                    url: "https://pubmed.ncbi.nlm.nih.gov/1/".into(),
                }],
                ..Default::default()
            };
            let GoldenRun {
                fixture,
                run,
                store,
                _tmp,
            } = run_fixture(
                include_str!("../tests/fixtures/golden_case_evidence.json"),
                sample_verdict_json_with_x_ref(),
                options,
            )
            .await;

            assert_eq!(run.trace.online_evidence_refs, vec!["X1"]);
            assert_eq!(run.verdict.applied_evidence[0].reference, "X1");
            for pii in &fixture.pii {
                assert!(!run.case.masked_text.contains(pii.as_str()));
            }
            let audit = store
                .lock()
                .unwrap()
                .latest_audit_for_case(&run.case.id)
                .unwrap()
                .unwrap();
            assert_eq!(audit.online_evidence_refs, vec!["X1"]);
        }
    }

    #[tokio::test]
    async fn local_only_rejects_network_provider_before_calling_llm() {
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
        let provider: Arc<dyn LlmProvider> = Arc::new(NetworkProvider);
        let pipeline = VerdictPipeline::new(
            sample_workspace(),
            Box::new(PipelineDeidentifier::new()),
            Arc::new(MockEmbedder::new()),
            repo,
            provider,
            store,
        );
        let options = VerdictOptions {
            data_boundary_mode: DataBoundaryMode::LocalOnly,
            ..Default::default()
        };

        let err = pipeline
            .run("Paciente con disnea.", "Manejo?", &[], &options)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn local_only_rejects_external_evidence_even_with_local_provider() {
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
        let options = VerdictOptions {
            data_boundary_mode: DataBoundaryMode::LocalOnly,
            external_evidence: vec![EvidenceItem {
                source: "europepmc".into(),
                id: "1".into(),
                title: "External paper".into(),
                authors: Vec::new(),
                year: None,
                venue: None,
                abstract_text: None,
                url: "https://example.test/1".into(),
            }],
            ..Default::default()
        };

        let err = pipeline
            .run("Paciente con disnea.", "Manejo?", &[], &options)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidConfig(_)));
    }
}
