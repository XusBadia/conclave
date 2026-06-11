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
    /// (`E1` workspace KB, `A1` case attachment, `X1` external evidence,
    /// `P1` past case).
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
    /// Citation key — must match one of the `[E*]`, `[A*]`, `[X*]`,
    /// `[P*]` ids supplied in the prompt.
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

/// JSON Schema mirroring [`Verdict`], passed as
/// `CompletionRequest::json_schema` on JSON-mode calls.
///
/// Most providers only check the field's *presence* to flip their
/// generic JSON mode on, but the claude-cli provider forwards the
/// schema verbatim to `claude -p --json-schema`, where the CLI's
/// structured-output tool **enforces** it. The enforced shape stops
/// the failure mode where the model double-encodes a nested container
/// (e.g. `"alternatives": "[…]"` — a string holding serialised JSON —
/// observed live with Sonnet 4.6), which `validate_verdict` would
/// only catch after the full four-phase run had already been paid for.
///
/// Keep in sync with the structs above; the
/// `schema_matches_serialized_verdict` test guards drift.
#[must_use]
pub fn verdict_json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "case_summary",
            "key_clinical_data",
            "applied_evidence",
            "primary_recommendation",
            "alternatives",
            "certainty_level",
            "certainty_justification",
            "red_flags",
            "follow_up_triggers",
            "disclaimer"
        ],
        "properties": {
            "case_summary": { "type": "string" },
            "key_clinical_data": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["label", "value"],
                    "properties": {
                        "label": { "type": "string" },
                        "value": { "type": "string" }
                    }
                }
            },
            "applied_evidence": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["ref", "claim"],
                    "properties": {
                        "ref": { "type": "string" },
                        "claim": { "type": "string" }
                    }
                }
            },
            "primary_recommendation": {
                "type": "object",
                "additionalProperties": false,
                "required": ["action", "rationale"],
                "properties": {
                    "action": { "type": "string" },
                    "rationale": { "type": "string" }
                }
            },
            "alternatives": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["action", "when_to_consider"],
                    "properties": {
                        "action": { "type": "string" },
                        "when_to_consider": { "type": "string" }
                    }
                }
            },
            "certainty_level": { "type": "string", "enum": ["high", "medium", "low"] },
            "certainty_justification": { "type": "string" },
            "red_flags": { "type": "array", "items": { "type": "string" } },
            "follow_up_triggers": { "type": "array", "items": { "type": "string" } },
            "disclaimer": { "type": "string" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard against drift between the Rust structs and the JSON
    /// Schema handed to structured-output providers: a fully-populated
    /// `Verdict` must serialise to exactly the keys the schema
    /// declares (all required, none unknown), with matching nested
    /// object keys.
    #[test]
    fn schema_matches_serialized_verdict() {
        let v = Verdict {
            case_summary: "s".into(),
            key_clinical_data: vec![KeyValue {
                label: "l".into(),
                value: "v".into(),
            }],
            applied_evidence: vec![EvidenceClaim {
                reference: "A1".into(),
                claim: "c".into(),
            }],
            primary_recommendation: Recommendation {
                action: "a".into(),
                rationale: "r".into(),
            },
            alternatives: vec![Alternative {
                action: "a".into(),
                when_to_consider: "w".into(),
            }],
            certainty_level: CertaintyLevel::Medium,
            certainty_justification: "j".into(),
            red_flags: vec!["f".into()],
            follow_up_triggers: vec!["t".into()],
            disclaimer: "d".into(),
        };
        let serialized = serde_json::to_value(&v).unwrap();
        let schema = verdict_json_schema();

        let props = schema["properties"].as_object().unwrap();
        let obj = serialized.as_object().unwrap();
        for key in obj.keys() {
            assert!(props.contains_key(key), "schema missing property `{key}`");
        }
        for key in props.keys() {
            assert!(
                obj.contains_key(key),
                "schema property `{key}` not produced by Verdict"
            );
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        for key in props.keys() {
            assert!(
                required.contains(&key.as_str()),
                "`{key}` missing from required"
            );
        }

        // Nested object property names must match the serde output too
        // (e.g. the `ref` rename on EvidenceClaim).
        let claim_keys: Vec<&str> = serialized["applied_evidence"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let schema_claim_props = schema["properties"]["applied_evidence"]["items"]["properties"]
            .as_object()
            .unwrap();
        for k in claim_keys {
            assert!(
                schema_claim_props.contains_key(k),
                "claim key `{k}` missing"
            );
        }

        // The enum casing must match serde's `lowercase` rename.
        assert_eq!(serialized["certainty_level"], "medium");
        assert!(schema["properties"]["certainty_level"]["enum"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("medium")));
    }
}
