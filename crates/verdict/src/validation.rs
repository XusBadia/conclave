//! Post-LLM validation: JSON shape + citation sanity.

use std::collections::HashSet;

use crate::schema::Verdict;

/// Validation errors surfaced by [`validate_verdict`].
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// The model's response did not parse as JSON or did not match the
    /// `Verdict` schema.
    #[error("schema validation failed: {0}")]
    Schema(String),
    /// One or more `applied_evidence[].ref` values point at ids that were
    /// not supplied in the prompt.
    #[error("invalid citations: {0:?}")]
    UnknownCitations(Vec<String>),
}

/// Parse the model's raw text into a [`Verdict`] and verify every cited
/// ref appears in the supplied id set.
pub fn validate_verdict(
    raw_json: &str,
    allowed_refs: &HashSet<String>,
) -> Result<Verdict, ValidationError> {
    let trimmed = strip_code_fences(raw_json);
    let verdict: Verdict =
        serde_json::from_str(trimmed).map_err(|e| ValidationError::Schema(e.to_string()))?;

    let mut bad = Vec::new();
    for claim in &verdict.applied_evidence {
        if !allowed_refs.contains(&claim.reference) {
            bad.push(claim.reference.clone());
        }
    }
    if !bad.is_empty() {
        return Err(ValidationError::UnknownCitations(bad));
    }
    Ok(verdict)
}

/// Some providers wrap their JSON in triple-backtick fences. Strip them so
/// `serde_json` can parse cleanly.
fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim().trim_end_matches("```").trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim().trim_end_matches("```").trim()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn sample_json(refs: &[&str]) -> String {
        let applied = refs
            .iter()
            .map(|r| format!(r#"{{"ref":"{r}","claim":"x"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
              "case_summary": "x",
              "key_clinical_data": [],
              "applied_evidence": [{applied}],
              "primary_recommendation": {{"action": "a", "rationale": "r"}},
              "certainty_level": "medium",
              "certainty_justification": "j",
              "red_flags": [],
              "follow_up_triggers": [],
              "disclaimer": "d"
            }}"#
        )
    }

    #[test]
    fn parses_clean_json() {
        let json = sample_json(&["E1"]);
        let allowed = allowed(&["E1"]);
        let verdict = validate_verdict(&json, &allowed).unwrap();
        assert_eq!(verdict.applied_evidence.len(), 1);
    }

    #[test]
    fn rejects_unknown_citation() {
        let json = sample_json(&["E1", "E2"]);
        let allowed = allowed(&["E1"]);
        let err = validate_verdict(&json, &allowed).unwrap_err();
        assert!(matches!(err, ValidationError::UnknownCitations(refs) if refs == vec!["E2"]));
    }

    #[test]
    fn strips_code_fences() {
        let json = format!("```json\n{}\n```", sample_json(&["E1"]));
        let verdict = validate_verdict(&json, &allowed(&["E1"])).unwrap();
        assert_eq!(verdict.primary_recommendation.action, "a");
    }

    #[test]
    fn schema_error_for_malformed_input() {
        let err = validate_verdict("not json", &allowed(&[])).unwrap_err();
        assert!(matches!(err, ValidationError::Schema(_)));
    }
}
