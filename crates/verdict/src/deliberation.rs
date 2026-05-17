//! Multi-pass committee deliberation.
//!
//! The default `VerdictPipeline::run` does a single LLM call. That's fine
//! for a quick second opinion, but for tough cases a clinician wants the
//! committee to **deliberate**: read everything carefully, draft a
//! recommendation, criticise it, then refine.
//!
//! This module implements that with four sequential LLM calls:
//!
//! 1. **Briefing** — free-form markdown: what we have, what's missing,
//!    salient findings, red-flag candidates. Vision-capable providers
//!    receive the case images here so they can interpret them once,
//!    surface the relevant observations in the briefing text, and the
//!    later phases work from text only.
//! 2. **Drafting** — first JSON verdict, citing only supplied evidence.
//! 3. **Red-team** — adversarial critique of the draft: what's wrong,
//!    what's missing, alternative interpretations, certainty pushback.
//! 4. **Finalize** — consolidated JSON verdict that takes the critique
//!    into account. This is the output that gets validated and persisted.
//!
//! Progress is streamed to the caller via an `mpsc::UnboundedSender`. Each
//! phase emits at minimum `Started` + `Completed` events; partial token
//! streaming is left to a future enhancement (current providers are
//! one-shot).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use conclave_core::{Error, Result, MEDICAL_DISCLAIMER};
use conclave_providers::{CompletionRequest, ImageInput, LlmProvider, Message};

use crate::persistence::{CaseAttachment, DeliberationTrace};
use crate::prompt::{
    CaseAttachmentInput, EvidenceChunkInput, PastCaseInput, PromptInputs, PromptTemplate,
};
use crate::schema::Verdict;
use crate::validation::validate_verdict;

/// Logical phases of the committee deliberation, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliberationPhase {
    Briefing,
    Drafting,
    RedTeam,
    Finalize,
}

impl DeliberationPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Briefing => "briefing",
            Self::Drafting => "drafting",
            Self::RedTeam => "redteam",
            Self::Finalize => "finalize",
        }
    }

    /// Maximum output tokens to request per phase. Tuned conservatively —
    /// the briefing/redteam are markdown and tend to be shorter; drafting
    /// and finalize emit the JSON verdict and need more headroom.
    pub const fn default_max_tokens(self) -> u32 {
        match self {
            Self::Briefing | Self::RedTeam => 1_500,
            Self::Drafting | Self::Finalize => 2_500,
        }
    }
}

/// Event surfaced as the deliberation progresses. The runtime channel
/// owner translates these into Tauri events / UI updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliberationEvent {
    /// Phase i/4 has started.
    PhaseStarted { phase: DeliberationPhase },
    /// Phase i/4 finished successfully with the full output text.
    PhaseCompleted {
        phase: DeliberationPhase,
        output: String,
    },
    /// Phase i/4 failed. The orchestrator surfaces the same error through
    /// its `Result` return, but a frontend listening to events can colour
    /// the failed phase before the awaiting code resolves.
    PhaseFailed {
        phase: DeliberationPhase,
        error: String,
    },
    /// All phases finished; the runtime owner can now persist + display.
    Done { verdict_json: String },
}

/// Inputs needed to run a deliberation. Stable across phases so the
/// orchestrator can interleave the same evidence in different prompts.
#[derive(Debug, Clone)]
pub struct DeliberationInputs {
    pub specialty: String,
    pub output_language: String,
    pub rules_block: String,
    pub masked_case_text: String,
    pub user_question: String,
    pub evidence_chunks: Vec<DeliberationEvidence>,
    pub past_cases: Vec<DeliberationPastCase>,
    pub attachments: Vec<CaseAttachment>,
    /// Optional images already loaded as base64 + media_type, ready to be
    /// forwarded to vision-capable providers. Empty for OCR-only mode.
    pub images: Vec<ImageInput>,
}

#[derive(Debug, Clone)]
pub struct DeliberationEvidence {
    pub document_title: String,
    pub doc_type: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct DeliberationPastCase {
    pub feedback: String,
    pub feedback_reason: String,
    pub case_summary: String,
    pub verdict_summary: String,
}

#[derive(Debug, Clone)]
pub struct DeliberationOptions {
    pub temperature: f32,
    pub include_images_in_vision: bool,
}

impl Default for DeliberationOptions {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            include_images_in_vision: true,
        }
    }
}

/// Full deliberation outcome. The caller wraps the `verdict` into the
/// usual `VerdictRecord` + persists `trace` alongside it.
#[derive(Debug, Clone)]
pub struct DeliberationOutcome {
    pub verdict: Verdict,
    pub trace: DeliberationTrace,
    pub final_response_text: String,
    pub model: String,
}

/// Run the four-phase deliberation. Each phase calls `provider.complete()`
/// once. The intermediate outputs are streamed via `events` and also
/// persisted in the returned `DeliberationTrace`.
pub async fn run_deliberation(
    provider: Arc<dyn LlmProvider>,
    inputs: DeliberationInputs,
    allowed_refs: HashSet<String>,
    options: DeliberationOptions,
    events: UnboundedSender<DeliberationEvent>,
) -> Result<DeliberationOutcome> {
    let started_at: DateTime<Utc> = Utc::now();
    let start = Instant::now();
    let caps = provider.capabilities();
    let vision_used = options.include_images_in_vision && caps.vision && !inputs.images.is_empty();

    let mut total_in: u32 = 0;
    let mut total_out: u32 = 0;
    let mut model: String;

    // ---------- Phase 1: Briefing (vision-aware) ----------
    let briefing_prompt = render_briefing_prompt(&inputs);
    let _ = events.send(DeliberationEvent::PhaseStarted {
        phase: DeliberationPhase::Briefing,
    });
    let briefing_resp = call_phase(
        &provider,
        &briefing_prompt,
        DeliberationPhase::Briefing,
        options.temperature,
        if vision_used {
            inputs.images.clone()
        } else {
            Vec::new()
        },
        false,
    )
    .await
    .inspect_err(|e| {
        let _ = events.send(DeliberationEvent::PhaseFailed {
            phase: DeliberationPhase::Briefing,
            error: e.to_string(),
        });
    })?;
    total_in += briefing_resp.usage.input_tokens;
    total_out += briefing_resp.usage.output_tokens;
    model = briefing_resp.model.clone();
    let briefing_text = briefing_resp.text.clone();
    let _ = events.send(DeliberationEvent::PhaseCompleted {
        phase: DeliberationPhase::Briefing,
        output: briefing_text.clone(),
    });

    // ---------- Phase 2: Drafting (JSON) ----------
    let drafting_prompt = render_drafting_prompt(&inputs, &briefing_text);
    let _ = events.send(DeliberationEvent::PhaseStarted {
        phase: DeliberationPhase::Drafting,
    });
    let draft_resp = call_phase(
        &provider,
        &drafting_prompt,
        DeliberationPhase::Drafting,
        options.temperature,
        Vec::new(),
        true,
    )
    .await
    .inspect_err(|e| {
        let _ = events.send(DeliberationEvent::PhaseFailed {
            phase: DeliberationPhase::Drafting,
            error: e.to_string(),
        });
    })?;
    total_in += draft_resp.usage.input_tokens;
    total_out += draft_resp.usage.output_tokens;
    let draft_text = draft_resp.text.clone();
    let _ = events.send(DeliberationEvent::PhaseCompleted {
        phase: DeliberationPhase::Drafting,
        output: draft_text.clone(),
    });

    // ---------- Phase 3: Red-team ----------
    let redteam_prompt = render_redteam_prompt(&inputs, &briefing_text, &draft_text);
    let _ = events.send(DeliberationEvent::PhaseStarted {
        phase: DeliberationPhase::RedTeam,
    });
    let redteam_resp = call_phase(
        &provider,
        &redteam_prompt,
        DeliberationPhase::RedTeam,
        // Slightly higher temperature for adversarial mode — we want
        // alternative interpretations, not the same answer.
        options.temperature.max(0.35),
        Vec::new(),
        false,
    )
    .await
    .inspect_err(|e| {
        let _ = events.send(DeliberationEvent::PhaseFailed {
            phase: DeliberationPhase::RedTeam,
            error: e.to_string(),
        });
    })?;
    total_in += redteam_resp.usage.input_tokens;
    total_out += redteam_resp.usage.output_tokens;
    let redteam_text = redteam_resp.text.clone();
    let _ = events.send(DeliberationEvent::PhaseCompleted {
        phase: DeliberationPhase::RedTeam,
        output: redteam_text.clone(),
    });

    // ---------- Phase 4: Finalize ----------
    let finalize_prompt =
        render_finalize_prompt(&inputs, &briefing_text, &draft_text, &redteam_text);
    let _ = events.send(DeliberationEvent::PhaseStarted {
        phase: DeliberationPhase::Finalize,
    });
    let final_resp = call_phase(
        &provider,
        &finalize_prompt,
        DeliberationPhase::Finalize,
        options.temperature,
        Vec::new(),
        true,
    )
    .await
    .inspect_err(|e| {
        let _ = events.send(DeliberationEvent::PhaseFailed {
            phase: DeliberationPhase::Finalize,
            error: e.to_string(),
        });
    })?;
    total_in += final_resp.usage.input_tokens;
    total_out += final_resp.usage.output_tokens;
    // The finalize phase carries the canonical model id we persist —
    // earlier phases may have run on a fallback if the provider rotated.
    if !final_resp.model.is_empty() {
        model = final_resp.model.clone();
    }
    let final_text = final_resp.text.clone();
    let _ = events.send(DeliberationEvent::PhaseCompleted {
        phase: DeliberationPhase::Finalize,
        output: final_text.clone(),
    });

    // Validate the final JSON. The drafting/finalize prompts already
    // restrict refs to the allowed set, but we still enforce it here so
    // hallucinated citations cannot slip through.
    let verdict = validate_verdict(&final_text, &allowed_refs)
        .map_err(|e| Error::Provider(format!("deliberation final verdict invalid: {e}")))?;
    let mut verdict = verdict;
    verdict.disclaimer = MEDICAL_DISCLAIMER.to_owned();

    let trace = DeliberationTrace {
        id: format!("delib-{}", Uuid::new_v4()),
        // Will be stamped to the real verdict id by the caller.
        verdict_id: String::new(),
        briefing_output: Some(briefing_text),
        drafting_output: Some(draft_text),
        redteam_output: Some(redteam_text),
        total_input_tokens: total_in,
        total_output_tokens: total_out,
        duration_ms: start.elapsed().as_millis() as u64,
        vision_used,
        created_at: started_at,
    };

    let _ = events.send(DeliberationEvent::Done {
        verdict_json: final_text.clone(),
    });

    Ok(DeliberationOutcome {
        verdict,
        trace,
        final_response_text: final_text,
        model,
    })
}

/// Run one LLM call for a single phase. `json_mode` toggles the request
/// JSON-mode hint (used for drafting + finalize).
async fn call_phase(
    provider: &Arc<dyn LlmProvider>,
    prompt: &str,
    phase: DeliberationPhase,
    temperature: f32,
    images: Vec<ImageInput>,
    json_mode: bool,
) -> Result<conclave_providers::CompletionResponse> {
    let req = CompletionRequest {
        model: String::new(),
        messages: vec![Message::user(prompt.to_owned())],
        max_output_tokens: Some(phase.default_max_tokens()),
        temperature: Some(temperature),
        json_schema: if json_mode {
            Some(serde_json::json!({"type": "object"}))
        } else {
            None
        },
        allow_web_search: false,
        images,
    };
    provider
        .complete(req)
        .await
        .map_err(|e| Error::Provider(format!("deliberation phase {} failed: {e}", phase.as_str())))
}

// ---------- Prompt rendering ----------

/// Helper: reuse the shared evidence/attachments/past-cases blocks from
/// `PromptTemplate` so the deliberation prompts look the same as the
/// quick prompt below the phase-specific instructions.
fn render_shared_evidence_block(inputs: &DeliberationInputs) -> String {
    let evidence_inputs: Vec<EvidenceChunkInput<'_>> = inputs
        .evidence_chunks
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
    let past_inputs: Vec<PastCaseInput<'_>> = inputs
        .past_cases
        .iter()
        .enumerate()
        .map(|(i, h)| PastCaseInput {
            index: i + 1,
            feedback: &h.feedback,
            feedback_reason: &h.feedback_reason,
            case_summary: &h.case_summary,
            previous_verdict_summary: &h.verdict_summary,
            user_modifications: "",
        })
        .collect();
    let attachment_inputs: Vec<CaseAttachmentInput<'_>> = inputs
        .attachments
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
    let prompt_inputs = PromptInputs {
        specialty: &inputs.specialty,
        output_language: &inputs.output_language,
        rules_block: &inputs.rules_block,
        evidence_chunks: &evidence_inputs,
        external_evidence: &[],
        past_cases: &past_inputs,
        case_attachments: &attachment_inputs,
        de_identified_case_text: &inputs.masked_case_text,
        user_question: &inputs.user_question,
        disclaimer: MEDICAL_DISCLAIMER,
    };
    PromptTemplate.render(&prompt_inputs)
}

fn render_briefing_prompt(inputs: &DeliberationInputs) -> String {
    let shared = render_shared_evidence_block(inputs);
    let mut out = String::with_capacity(shared.len() + 1024);
    out.push_str(
        "You are the lead clinician of a multidisciplinary virtual board.\n\
This is the BRIEFING phase — the first of four. The remaining phases \
(drafting, red-team, finalize) build directly on what you write here.\n\n\
Read every supplied source carefully, including any images attached to \
this turn. Produce a structured **markdown briefing** — NOT JSON, NOT a \
recommendation. The briefing must contain these sections:\n\n\
1. **What we have** — bullet list of every salient finding, grouped by \
source ([E*], [A*], [P*]). Quote evidence ids inline so later phases can \
trace each point.\n\
2. **What we're missing** — data that would change the management.\n\
3. **Preliminary red flags** — anything that warrants escalation or a \
worst-case differential.\n\
4. **Image observations** — only if images were provided in this turn. \
Describe what you actually see on each [A*] image (rhythm, axis, ST \
changes for ECGs; obvious findings for X-rays; etc.). If no image was \
provided, write \"(no images supplied)\".\n\n\
Do NOT propose a verdict yet. Be precise, terse, and label every claim \
with its evidence id. Output language: ",
    );
    out.push_str(&inputs.output_language);
    out.push_str(".\n\n");
    out.push_str(&shared);
    out
}

fn render_drafting_prompt(inputs: &DeliberationInputs, briefing: &str) -> String {
    let shared = render_shared_evidence_block(inputs);
    format!(
        "You are the lead clinician. This is the DRAFTING phase.\n\n\
A peer briefing has already been produced for this case (see BRIEFING \
below). Use it as a high-level digest, but defer to the EVIDENCE / CASE \
ATTACHMENTS blocks for the authoritative content.\n\n\
Produce a JSON verdict that strictly matches the schema described in the \
EVIDENCE block. Cite only the supplied evidence ids ([E*], [A*], [P*]). \
Treat this as a first draft — the next phase will challenge it.\n\n\
BRIEFING\n========\n{briefing}\n\n{shared}",
    )
}

fn render_redteam_prompt(inputs: &DeliberationInputs, briefing: &str, draft_json: &str) -> String {
    let shared = render_shared_evidence_block(inputs);
    format!(
        "You are an adversarial reviewer on the same virtual board. This is the \
RED-TEAM phase.\n\n\
A draft verdict has been produced. Your job: find what's wrong with it.\n\n\
Produce a **markdown critique** (NOT JSON) with these sections:\n\n\
1. **Diagnostic alternatives missed** — competing diagnoses the draft did \
not consider, with a one-line rationale each.\n\
2. **Evidence misuse** — places where the draft over-reaches what the \
supplied evidence actually says, or misquotes an evidence id.\n\
3. **Certainty pushback** — argue whether the certainty_level is \
honest. If the draft says \"high\" but only one evidence source supports \
it, say so.\n\
4. **Safety gaps** — red flags or follow-up triggers the draft missed.\n\
5. **Verdict** — a single line: ACCEPT / MODIFY / REJECT, with a 1-line \
justification.\n\n\
Be terse and ruthless. If the draft is genuinely sound, say so plainly \
in section 5 (\"ACCEPT — draft is well-supported by [E1][A2]\"). Output \
language: {lang}.\n\n\
DRAFT VERDICT (JSON)\n====================\n{draft_json}\n\n\
BRIEFING (for reference)\n========================\n{briefing}\n\n{shared}",
        lang = inputs.output_language,
    )
}

fn render_finalize_prompt(
    inputs: &DeliberationInputs,
    briefing: &str,
    draft_json: &str,
    redteam: &str,
) -> String {
    let shared = render_shared_evidence_block(inputs);
    format!(
        "You are the lead clinician. This is the FINALIZE phase.\n\n\
You have a draft verdict and an adversarial critique of it. Produce the \
**final JSON verdict** that:\n\n\
- Incorporates the substantive points from the critique. Where the \
critique flagged certainty pushback, lower certainty_level. Where it \
flagged missed differentials, add them to `alternatives`. Where it \
flagged safety gaps, add them to `red_flags` / `follow_up_triggers`.\n\
- Keeps citing only the supplied evidence ids. Do NOT invent ids.\n\
- Returns the canonical JSON schema described in the EVIDENCE block — no \
extra text, no preface, just the JSON object.\n\n\
DRAFT VERDICT (JSON)\n====================\n{draft_json}\n\n\
RED-TEAM CRITIQUE (markdown)\n============================\n{redteam}\n\n\
BRIEFING (for reference)\n========================\n{briefing}\n\n{shared}",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use conclave_providers::MockProvider;

    fn sample_inputs() -> DeliberationInputs {
        DeliberationInputs {
            specialty: "cardiología".into(),
            output_language: "es".into(),
            rules_block: String::new(),
            masked_case_text: "Paciente con dolor torácico opresivo.".into(),
            user_question: "Manejo inicial?".into(),
            evidence_chunks: vec![DeliberationEvidence {
                document_title: "Guía SCA".into(),
                doc_type: "pdf".into(),
                snippet: "AAS 300mg + clopidogrel 600mg.".into(),
            }],
            past_cases: vec![],
            attachments: vec![],
            images: vec![],
        }
    }

    fn final_json() -> String {
        serde_json::json!({
            "case_summary": "Dolor torácico de probable origen isquémico.",
            "key_clinical_data": [],
            "applied_evidence": [{"ref": "E1", "claim": "Antiagregación dual"}],
            "primary_recommendation": {
                "action": "ECG urgente + troponinas seriadas + AAS 300mg.",
                "rationale": "Sospecha de SCA con elevación de riesgo."
            },
            "alternatives": [],
            "certainty_level": "medium",
            "certainty_justification": "Datos clínicos sugestivos pero no confirmatorios.",
            "red_flags": [],
            "follow_up_triggers": [],
            "disclaimer": "x"
        })
        .to_string()
    }

    #[tokio::test]
    async fn four_phase_run_emits_events_and_returns_verdict() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            "Briefing markdown".into(),
            r#"{"case_summary":"draft","key_clinical_data":[],"applied_evidence":[],"primary_recommendation":{"action":"a","rationale":"r"},"alternatives":[],"certainty_level":"low","certainty_justification":"j","red_flags":[],"follow_up_triggers":[],"disclaimer":"x"}"#.into(),
            "Red-team critique".into(),
            final_json(),
        ]));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let inputs = sample_inputs();
        let mut allowed: HashSet<String> = HashSet::new();
        allowed.insert("E1".to_owned());

        let outcome = run_deliberation(
            provider,
            inputs,
            allowed,
            DeliberationOptions::default(),
            tx,
        )
        .await
        .unwrap();

        // Verdict was the FINAL response, not the draft.
        assert!(outcome.verdict.case_summary.contains("isquémico"));
        assert_eq!(outcome.verdict.applied_evidence.len(), 1);
        // Trace captured the three intermediate outputs.
        assert_eq!(
            outcome.trace.briefing_output.as_deref(),
            Some("Briefing markdown"),
        );
        assert!(outcome
            .trace
            .drafting_output
            .as_deref()
            .unwrap()
            .contains("draft"));
        assert_eq!(
            outcome.trace.redteam_output.as_deref(),
            Some("Red-team critique"),
        );
        assert!(!outcome.trace.vision_used);

        // Collect every event the orchestrator emitted.
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        // 4 phases × (Started + Completed) + Done = 9 events.
        assert_eq!(events.len(), 9);
        assert!(matches!(
            events.last(),
            Some(DeliberationEvent::Done { .. })
        ));
    }

    #[tokio::test]
    async fn invalid_final_json_surfaces_provider_error() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            "briefing".into(),
            "draft".into(),
            "redteam".into(),
            "this is not json".into(),
        ]));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let inputs = sample_inputs();
        let err = run_deliberation(
            provider,
            inputs,
            HashSet::new(),
            DeliberationOptions::default(),
            tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Provider(_)));
    }

    #[tokio::test]
    async fn vision_used_flag_reflects_capability_and_images() {
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![
            "briefing".into(),
            final_json(),
            "redteam".into(),
            final_json(),
        ]));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut inputs = sample_inputs();
        inputs.images.push(ImageInput {
            media_type: "image/png".into(),
            base64_data: "iVBORw0KGgo=".into(),
        });
        // MockProvider advertises vision=false, so vision_used must be false
        // even though images were supplied.
        let mut allowed: HashSet<String> = HashSet::new();
        allowed.insert("E1".to_owned());
        let outcome = run_deliberation(
            provider,
            inputs,
            allowed,
            DeliberationOptions::default(),
            tx,
        )
        .await
        .unwrap();
        assert!(!outcome.trace.vision_used);
    }
}
