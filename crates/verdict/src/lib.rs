#![allow(
    clippy::option_if_let_else,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::module_name_repetitions,
    clippy::missing_const_for_fn,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value,
    clippy::implicit_hasher,
    clippy::large_stack_arrays,
    clippy::large_enum_variant,
    clippy::struct_field_names,
    clippy::needless_continue,
    clippy::items_after_statements,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::map_unwrap_or,
    clippy::bool_assert_comparison,
    clippy::missing_const_for_thread_local,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports,
    clippy::redundant_clone,
    clippy::assigning_clones,
    clippy::unused_self,
    clippy::format_push_string
)]

//! Verdict engine — Phase 4.
//!
//! Glues together the per-workspace knowledge base ([`conclave_rag`]),
//! the PII pipeline ([`conclave_deident`]) and the configured LLM
//! provider ([`conclave_providers`]) into a single `VerdictPipeline`
//! that takes a clinical case in and emits a structured, schema-valid
//! [`Verdict`] out.
//!
//! ## Privacy invariants
//!
//! - Every case is run through the de-identifier before any LLM call. The
//!   pipeline holds the original text in memory only long enough to mask
//!   it and compute a fingerprint. New finalized runs persist masked text
//!   plus hashes by default, not raw clinical narrative.
//! - The disclaimer field is copied verbatim from
//!   [`conclave_core::MEDICAL_DISCLAIMER`] regardless of what the model
//!   produced, so we control the legal footer at all times.

pub mod attachments;
pub mod clinical_services;
pub mod connectors;
pub mod deliberation;
pub mod eval;
pub mod persistence;
pub mod pipeline;
pub mod priors;
pub mod privacy;
pub mod prompt;
pub mod qa;
pub mod schema;
pub mod skills;
pub mod validation;
pub mod workflows;

pub use attachments::ingest_case_attachments;
pub use clinical_services::{
    CsvTerminologyService, DeidentService, EvidenceService, ExternalEvidenceHit, ExtractionHit,
    ExtractionService, LocalDeidentService, TerminologyHit, TerminologyService,
};
pub use connectors::{
    ConnectorAuditEvent, ConnectorAuth, ConnectorCallPlan, ConnectorCallRequest, ConnectorConfig,
    ConnectorLoggingPolicy, ConnectorRegistry,
};
pub use deliberation::{
    run_deliberation, DeliberationEvent, DeliberationEvidence, DeliberationInputs,
    DeliberationOptions, DeliberationOutcome, DeliberationPastCase, DeliberationPhase,
};
pub use eval::{
    CaseOutcome, Concordance, ConcordanceReport, DecisionCategory, StratifiedConcordance, Tally,
};
pub use persistence::{
    AuditRunRecord, AuditStatus, CaseAttachment, CaseRecord, CaseStatus, CaseStore,
    DeliberationTrace, ExportedCase, ExportedFeedback, FeedbackKind, FeedbackRecord, PastCaseHit,
    RetrievalTrace, ReviewDecision, ReviewMetadataRecord, StoreStats, VerdictRecord,
};
pub use pipeline::{VerdictOptions, VerdictPipeline, VerdictRun};
pub use priors::load_specialty_priors;
pub use privacy::{sha256_hex, AuditPayloadMode, DataBoundaryMode, RawTextRetention};
pub use prompt::{
    CaseAttachmentInput, EvidenceChunkInput, ExternalEvidenceInput, PastCaseInput, PromptInputs,
    PromptTemplate, VERDICT_PROMPT_VERSION,
};
pub use qa::{QaPipeline, QaResponse, QaSource};
pub use schema::{
    CertaintyLevel, DataCompleteness, EvidenceClaim, KeyValue, Recommendation, Verdict,
};
pub use skills::{load_skill, load_skills, Skill, SkillSource};
pub use validation::{validate_verdict, ValidationError};
pub use workflows::{run_workflow, ClinicalWorkflow, WorkflowOutput};
