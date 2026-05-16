//! Serializable schema the LLM is asked to fill. Validated by
//! `validation::validate_verdict`.

use serde::{Deserialize, Serialize};

/// Top-level verdict object — exactly the shape documented in
/// `PROMPTING.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// Three-to-five sentence executive summary of the case.
    pub case_summary: String,
    /// Bullet-list of the clinically-relevant data points the model used.
    pub key_clinical_data: Vec<KeyValue>,
    /// Claims with explicit citations to provided evidence ids
    /// (`E1`, `X1`, `P1`).
    pub applied_evidence: Vec<EvidenceClaim>,
    /// Top recommendation with rationale.
    pub primary_recommendation: Recommendation,
    /// Alternative paths and when to consider them.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// Self-reported confidence level.
    pub certainty_level: CertaintyLevel,
    /// Justification for the confidence level.
    pub certainty_justification: String,
    /// Hard contraindications, missing data, escalation triggers.
    #[serde(default)]
    pub red_flags: Vec<String>,
    /// Conditions that should prompt re-evaluation.
    #[serde(default)]
    pub follow_up_triggers: Vec<String>,
    /// Standard Conclave disclaimer (overwritten by the pipeline so it
    /// always matches the canonical text).
    pub disclaimer: String,
}

/// Generic label/value used in `key_clinical_data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValue {
    pub label: String,
    pub value: String,
}

/// A claim grounded in one of the provided evidence ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceClaim {
    /// Citation key — must match one of the `[E*]`, `[X*]`, `[P*]` ids
    /// supplied in the prompt.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The claim derived from that evidence.
    pub claim: String,
}

/// Primary or alternative recommendation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    /// What to do.
    pub action: String,
    /// Why this is recommended.
    pub rationale: String,
}

/// An alternative path with the trigger to consider it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    /// What to do instead.
    pub action: String,
    /// Condition that would make this alternative the right one.
    pub when_to_consider: String,
}

/// Three-level confidence flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CertaintyLevel {
    High,
    Medium,
    Low,
}

impl CertaintyLevel {
    /// Short human label used by the CLI renderer.
    pub const fn label(self) -> &'static str {
        match self {
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }
}
