//! Deterministic clinical workflows over an already generated verdict.
//!
//! These workflows do not call an LLM. They reshape the structured verdict
//! and de-identified case text into reviewable artifacts that remain local
//! and reproducible.

use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

use crate::persistence::CaseRecord;
use crate::schema::Verdict;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClinicalWorkflow {
    ChartSummary,
    MedRecDiscrepancy,
    GuidelineReview,
    DischargeHandoff,
    CodingAudit,
    StructuredExtractionFhirDiff,
}

impl ClinicalWorkflow {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "chart_summary" => Some(Self::ChartSummary),
            "med_rec_discrepancy" => Some(Self::MedRecDiscrepancy),
            "guideline_review" => Some(Self::GuidelineReview),
            "discharge_handoff" => Some(Self::DischargeHandoff),
            "coding_audit" => Some(Self::CodingAudit),
            "structured_extraction_fhir_diff" => Some(Self::StructuredExtractionFhirDiff),
            _ => None,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::ChartSummary => "chart_summary",
            Self::MedRecDiscrepancy => "med_rec_discrepancy",
            Self::GuidelineReview => "guideline_review",
            Self::DischargeHandoff => "discharge_handoff",
            Self::CodingAudit => "coding_audit",
            Self::StructuredExtractionFhirDiff => "structured_extraction_fhir_diff",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowOutput {
    pub workflow: ClinicalWorkflow,
    pub title: String,
    pub markdown: String,
}

pub fn run_workflow(
    workflow: ClinicalWorkflow,
    case: &CaseRecord,
    verdict: &Verdict,
    terminology_available: bool,
) -> Result<WorkflowOutput> {
    let markdown = match workflow {
        ClinicalWorkflow::ChartSummary => chart_summary(case, verdict),
        ClinicalWorkflow::MedRecDiscrepancy => med_rec_discrepancy(case),
        ClinicalWorkflow::GuidelineReview => guideline_review(verdict),
        ClinicalWorkflow::DischargeHandoff => discharge_handoff(case, verdict),
        ClinicalWorkflow::CodingAudit if terminology_available => coding_audit(verdict),
        ClinicalWorkflow::CodingAudit => {
            return Err(Error::invalid_config(
                "coding_audit requires a local terminology CSV catalog",
            ));
        }
        ClinicalWorkflow::StructuredExtractionFhirDiff => {
            return Err(Error::Unimplemented(
                "structured_extraction_fhir_diff is reserved for a validated FHIR extraction phase",
            ));
        }
    };
    Ok(WorkflowOutput {
        workflow,
        title: workflow.id().replace('_', " "),
        markdown,
    })
}

fn chart_summary(case: &CaseRecord, verdict: &Verdict) -> String {
    format!(
        "# Chart summary\n\n\
Case: `{}`\n\n\
Question: {}\n\n\
Summary: {}\n\n\
Recommendation: {}\n\n\
Certainty: {:?} — {}\n",
        case.id,
        case.question,
        verdict.case_summary,
        verdict.primary_recommendation.action,
        verdict.certainty_level,
        verdict.certainty_justification
    )
}

fn med_rec_discrepancy(case: &CaseRecord) -> String {
    let mut lines = Vec::new();
    for line in case.masked_text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("mg")
            || lower.contains("medic")
            || lower.contains("tratamiento")
            || lower.contains("dose")
            || lower.contains("tablet")
        {
            lines.push(format!("- {}", line.trim()));
        }
    }
    if lines.is_empty() {
        lines.push("- No medication-like lines were detected in the de-identified text.".into());
    }
    format!(
        "# Medication reconciliation discrepancy scan\n\n\
Potential medication lines for clinician review:\n\n{}\n\n\
No medication change is implied by this scan.",
        lines.join("\n")
    )
}

fn guideline_review(verdict: &Verdict) -> String {
    let evidence = if verdict.applied_evidence.is_empty() {
        "- No explicit evidence claims were cited.".to_owned()
    } else {
        verdict
            .applied_evidence
            .iter()
            .map(|e| format!("- [{}] {}", e.reference, e.claim))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "# Guideline-grounded review\n\n\
Primary recommendation:\n{}\n\n\
Evidence claims:\n{}\n\n\
Uncertainty:\n{}\n\n\
Red flags:\n{}\n",
        verdict.primary_recommendation.action,
        evidence,
        verdict.certainty_justification,
        list_or_none(&verdict.red_flags)
    )
}

fn discharge_handoff(case: &CaseRecord, verdict: &Verdict) -> String {
    format!(
        "# Discharge / handoff draft\n\n\
Situation: {}\n\n\
Background: {}\n\n\
Assessment: {}\n\n\
Recommendation: {}\n\n\
Follow-up triggers:\n{}\n",
        case.question,
        verdict.case_summary,
        verdict.certainty_justification,
        verdict.primary_recommendation.action,
        list_or_none(&verdict.follow_up_triggers)
    )
}

fn coding_audit(verdict: &Verdict) -> String {
    format!(
        "# Coding audit support\n\n\
Candidate clinical concepts to search in local terminology catalogs:\n\n\
- {}\n\n\
Supporting summary:\n{}\n",
        verdict.primary_recommendation.action, verdict.case_summary
    )
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- None documented.".to_owned()
    } else {
        items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_case() -> CaseRecord {
        CaseRecord {
            id: "c1".into(),
            created_at: Utc::now(),
            case_date: Utc::now(),
            workspace_id: "ws".into(),
            question: "Alta?".into(),
            original_text: String::new(),
            masked_text: "Tratamiento: aspirina 100 mg".into(),
            deident_pipeline_id: "p".into(),
            status: crate::CaseStatus::ReviewReady,
            patient_label: String::new(),
            latest_error: None,
            raw_text_sha256: String::new(),
            raw_text_retention: crate::RawTextRetention::Discarded,
        }
    }

    fn sample_verdict() -> Verdict {
        serde_json::from_value(serde_json::json!({
            "case_summary": "Resumen.",
            "key_clinical_data": [],
            "applied_evidence": [{"ref": "E1", "claim": "Claim"}],
            "primary_recommendation": {"action": "Plan", "rationale": "R"},
            "certainty_level": "low",
            "certainty_justification": "Faltan datos.",
            "red_flags": ["Alarma"],
            "follow_up_triggers": ["Control"],
            "disclaimer": "x"
        }))
        .unwrap()
    }

    #[test]
    fn deterministic_workflow_outputs_markdown() {
        let out = run_workflow(
            ClinicalWorkflow::GuidelineReview,
            &sample_case(),
            &sample_verdict(),
            false,
        )
        .unwrap();
        assert!(out.markdown.contains("[E1]"));
    }

    #[test]
    fn coding_audit_requires_terminology() {
        let err = run_workflow(
            ClinicalWorkflow::CodingAudit,
            &sample_case(),
            &sample_verdict(),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidConfig(_)));
    }
}
