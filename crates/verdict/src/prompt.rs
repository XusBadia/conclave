//! Prompt assembly. The template here is the canonical Phase 4 prompt
//! defined in `PROMPTING.md` and pinned to [`VERDICT_PROMPT_VERSION`].
//!
//! We use plain string substitution rather than a template engine because
//! the structure is simple and we want every prompt to be reproducible
//! byte-for-byte across runs.

/// Stable version string persisted alongside every generated verdict so we
/// can reproduce the exact prompt later if the template changes.
pub const VERDICT_PROMPT_VERSION: &str = "verdict_v1";

/// Inputs needed to assemble the verdict prompt.
#[derive(Debug, Clone)]
pub struct PromptInputs<'a> {
    /// Clinical specialty configured on the workspace.
    pub specialty: &'a str,
    /// Output language (ISO code or human name).
    pub output_language: &'a str,
    /// Workspace rules text, already formatted as bullet list.
    pub rules_block: &'a str,
    /// Evidence chunks retrieved from the knowledge base.
    pub evidence_chunks: &'a [EvidenceChunkInput<'a>],
    /// External evidence (Phase 6).
    pub external_evidence: &'a [ExternalEvidenceInput<'a>],
    /// Similar past cases (Phase 5).
    pub past_cases: &'a [PastCaseInput<'a>],
    /// The de-identified case text.
    pub de_identified_case_text: &'a str,
    /// User question (defaults to "What is the recommended management?").
    pub user_question: &'a str,
    /// Disclaimer text to inject in the schema instructions.
    pub disclaimer: &'a str,
}

/// One retrieved chunk fed into the EVIDENCE block.
#[derive(Debug, Clone)]
pub struct EvidenceChunkInput<'a> {
    /// 1-based index inside the EVIDENCE block.
    pub index: usize,
    /// Document title (or filename stem).
    pub document_title: &'a str,
    /// Page label — defaults to "1" when pages are unknown.
    pub page: &'a str,
    /// Document type (`pdf`, `docx`, …).
    pub doc_type: &'a str,
    /// Chunk text, already trimmed to a reasonable length.
    pub snippet: &'a str,
}

/// One external evidence item (PubMed / Europe PMC).
#[derive(Debug, Clone)]
pub struct ExternalEvidenceInput<'a> {
    pub index: usize,
    pub title: &'a str,
    pub authors: &'a str,
    pub year: &'a str,
    pub venue: &'a str,
    pub abstract_text: &'a str,
}

/// One past case included from workspace memory.
#[derive(Debug, Clone)]
pub struct PastCaseInput<'a> {
    pub index: usize,
    pub feedback: &'a str,
    pub feedback_reason: &'a str,
    pub case_summary: &'a str,
    pub previous_verdict_summary: &'a str,
    pub user_modifications: &'a str,
}

/// Canonical verdict prompt — sole template at v1.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptTemplate;

impl PromptTemplate {
    /// Render the prompt with the given inputs.
    pub fn render(self, inputs: &PromptInputs<'_>) -> String {
        let evidence = render_evidence(inputs.evidence_chunks);
        let external = render_external(inputs.external_evidence);
        let past_cases = render_past_cases(inputs.past_cases);
        let rules = if inputs.rules_block.trim().is_empty() {
            "No workspace rules defined."
        } else {
            inputs.rules_block
        };
        let question = if inputs.user_question.trim().is_empty() {
            "What is the recommended management?"
        } else {
            inputs.user_question
        };

        format!(
            "You are Conclave, a clinical decision support assistant operating as a \
multidisciplinary virtual board for {specialty}. You produce structured \
recommendations to support — never replace — the treating clinician.\n\n\
Your output is consumed by software and must validate against the provided \
JSON schema. Do not include any text outside the JSON object.\n\n\
Hard rules:\n\
- Use only the evidence supplied in the EVIDENCE and PAST_CASES blocks.\n\
  If you cite anything not present there, the response is invalid.\n\
- The case data has been de-identified. Do not invent personal details.\n\
- If the supplied information is insufficient for a confident answer, set \
certainty_level to \"low\" and list the missing data in red_flags.\n\
- Workspace rules (see RULES) are constraints. Violating a rule invalidates \
the response.\n\
- Output language: {output_language}.\n\n\
WORKSPACE RULES\n===============\n{rules}\n\n\
EVIDENCE (from this centre's knowledge base)\n============================================\n\
{evidence}\n\
EXTERNAL EVIDENCE (live literature, not validated by this centre)\n\
================================================================\n{external}\n\
PAST CASES (similar prior cases with user feedback)\n===================================================\n\
{past_cases}\n\
USER\n====\nCASE\n----\n{case}\n\n\
QUESTION\n--------\n{question}\n\n\
OUTPUT SCHEMA\n-------------\n\
Return a JSON object with exactly these keys:\n\n\
{{\n  \"case_summary\": string,\n  \"key_clinical_data\": [{{\"label\": string, \"value\": string}}],\n  \
\"applied_evidence\": [{{\"ref\": \"E1\"|\"X1\"|\"P1\", \"claim\": string}}],\n  \
\"primary_recommendation\": {{\"action\": string, \"rationale\": string}},\n  \
\"alternatives\": [{{\"action\": string, \"when_to_consider\": string}}],\n  \
\"certainty_level\": \"high\"|\"medium\"|\"low\",\n  \"certainty_justification\": string,\n  \
\"red_flags\": [string],\n  \"follow_up_triggers\": [string],\n  \"disclaimer\": string\n}}\n\n\
The \"disclaimer\" field must contain the standard Conclave disclaimer in {output_language}, \
taken verbatim:\n\n{disclaimer}\n",
            specialty = inputs.specialty,
            output_language = inputs.output_language,
            rules = rules,
            evidence = evidence,
            external = external,
            past_cases = past_cases,
            case = inputs.de_identified_case_text,
            question = question,
            disclaimer = inputs.disclaimer,
        )
    }
}

fn render_evidence(items: &[EvidenceChunkInput<'_>]) -> String {
    if items.is_empty() {
        return "(no evidence retrieved from the workspace)\n".to_owned();
    }
    let mut out = String::new();
    for ev in items {
        out.push_str(&format!(
            "[E{index}] source: \"{title}\", page {page}, type: {doc_type}\n{snippet}\n\n",
            index = ev.index,
            title = ev.document_title,
            page = ev.page,
            doc_type = ev.doc_type,
            snippet = ev.snippet.trim(),
        ));
    }
    out
}

fn render_external(items: &[ExternalEvidenceInput<'_>]) -> String {
    if items.is_empty() {
        return "(none — phase 6 not enabled for this call)\n".to_owned();
    }
    let mut out = String::new();
    for ev in items {
        out.push_str(&format!(
            "[X{index}] {title} ({authors}, {year}, {venue})\n{abstract_text}\n\n",
            index = ev.index,
            title = ev.title,
            authors = ev.authors,
            year = ev.year,
            venue = ev.venue,
            abstract_text = ev.abstract_text.trim(),
        ));
    }
    out
}

fn render_past_cases(items: &[PastCaseInput<'_>]) -> String {
    if items.is_empty() {
        return "(no similar past cases in this workspace yet)\n".to_owned();
    }
    let mut out = String::new();
    for pc in items {
        out.push_str(&format!(
            "[P{index}] feedback: {feedback} ({feedback_reason})\nCase summary: {case_summary}\nVerdict given: {previous_verdict_summary}\n",
            index = pc.index,
            feedback = pc.feedback,
            feedback_reason = pc.feedback_reason,
            case_summary = pc.case_summary,
            previous_verdict_summary = pc.previous_verdict_summary,
        ));
        if !pc.user_modifications.trim().is_empty() {
            out.push_str(&format!("User modifications: {}\n", pc.user_modifications));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_render() {
        let inputs = PromptInputs {
            specialty: "cardiología",
            output_language: "es",
            rules_block: "",
            evidence_chunks: &[],
            external_evidence: &[],
            past_cases: &[],
            de_identified_case_text: "Paciente con dolor torácico.",
            user_question: "",
            disclaimer: "Disclaimer test.",
        };
        let prompt = PromptTemplate.render(&inputs);
        assert!(prompt.contains("cardiología"));
        assert!(prompt.contains("Paciente con dolor torácico."));
        assert!(prompt.contains("No workspace rules defined."));
        assert!(prompt.contains("(no evidence retrieved"));
        assert!(prompt.contains("What is the recommended management?"));
    }

    #[test]
    fn evidence_is_numbered() {
        let chunks = vec![EvidenceChunkInput {
            index: 1,
            document_title: "Guía Clínica",
            page: "12",
            doc_type: "pdf",
            snippet: "Furosemida IV 20-40 mg.",
        }];
        let inputs = PromptInputs {
            specialty: "cardiología",
            output_language: "es",
            rules_block: "",
            evidence_chunks: &chunks,
            external_evidence: &[],
            past_cases: &[],
            de_identified_case_text: "case",
            user_question: "Manejo inicial?",
            disclaimer: "x",
        };
        let prompt = PromptTemplate.render(&inputs);
        assert!(prompt.contains("[E1]"));
        assert!(prompt.contains("Furosemida IV"));
        assert!(prompt.contains("Manejo inicial?"));
    }
}
