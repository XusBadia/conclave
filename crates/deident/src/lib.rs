//! De-identification of personally identifiable information (PII) in clinical
//! free-text.
//!
//! Phase 0 ships only the trait surface and a deliberately conservative
//! placeholder detector that masks digit sequences (a stand-in for
//! patient/episode numbers). Real detectors arrive in Phase 3.

use serde::{Deserialize, Serialize};

/// A category of PII recognised by the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Full or partial person names.
    Name,
    /// National ID / passport / NHS number / similar.
    GovId,
    /// Patient or episode identifiers internal to a hospital system.
    MedicalRecordNumber,
    /// Telephone numbers.
    Phone,
    /// Email addresses.
    Email,
    /// Postal or physical addresses.
    Address,
    /// Calendar dates.
    Date,
    /// A generic numeric identifier (catch-all used by the placeholder).
    NumericId,
}

/// A single PII span detected in a piece of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Byte offset where the entity starts (inclusive).
    pub start: usize,
    /// Byte offset where the entity ends (exclusive).
    pub end: usize,
    /// Category of the entity.
    pub kind: EntityKind,
    /// The original surface form, retained for audit purposes.
    pub surface: String,
}

/// Result of running a de-identification pass over a piece of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deidentified {
    /// Text with every detected entity replaced by a redaction token.
    pub redacted: String,
    /// All detected entities, in left-to-right order.
    pub entities: Vec<Entity>,
}

/// Trait every PII detector implements.
pub trait Deidentifier {
    /// Stable identifier (e.g. `"placeholder"`, `"regex-v1"`, `"presidio"`).
    fn id(&self) -> &'static str;

    /// Detect and redact PII in `text`.
    fn redact(&self, text: &str) -> conclave_core::Result<Deidentified>;
}

/// Placeholder detector that masks naive digit sequences of length >= 4.
///
/// This is intentionally a stand-in for real detection logic. Do **not** use
/// it as a sole safeguard for actual patient data.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlaceholderDeidentifier;

impl PlaceholderDeidentifier {
    /// Construct a new placeholder detector.
    pub const fn new() -> Self {
        Self
    }
}

impl Deidentifier for PlaceholderDeidentifier {
    fn id(&self) -> &'static str {
        "placeholder"
    }

    fn redact(&self, text: &str) -> conclave_core::Result<Deidentified> {
        let mut entities = Vec::new();
        let mut out = String::with_capacity(text.len());
        let bytes = text.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            if b.is_ascii_digit() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let end = i;
                let surface = &text[start..end];
                if surface.len() >= 4 {
                    entities.push(Entity {
                        start,
                        end,
                        kind: EntityKind::NumericId,
                        surface: surface.to_owned(),
                    });
                    out.push_str("[REDACTED:NUMERIC_ID]");
                } else {
                    out.push_str(surface);
                }
            } else {
                // Push the next UTF-8 character whole.
                let char_start = i;
                let ch = text[char_start..]
                    .chars()
                    .next()
                    .expect("non-empty remainder");
                let char_len = ch.len_utf8();
                out.push(ch);
                i += char_len;
            }
        }
        Ok(Deidentified {
            redacted: out,
            entities,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_long_digit_runs() {
        let d = PlaceholderDeidentifier::new();
        let out = d.redact("Patient MRN 1234567 is stable.").unwrap();
        assert_eq!(out.redacted, "Patient MRN [REDACTED:NUMERIC_ID] is stable.");
        assert_eq!(out.entities.len(), 1);
        assert_eq!(out.entities[0].surface, "1234567");
        assert_eq!(out.entities[0].kind, EntityKind::NumericId);
    }

    #[test]
    fn preserves_short_numbers() {
        let d = PlaceholderDeidentifier::new();
        let out = d.redact("Take 2 tablets, 3 times a day.").unwrap();
        assert_eq!(out.redacted, "Take 2 tablets, 3 times a day.");
        assert!(out.entities.is_empty());
    }

    #[test]
    fn handles_unicode_safely() {
        let d = PlaceholderDeidentifier::new();
        let out = d.redact("Niño con MRN 9876543 — ECG normal.").unwrap();
        assert!(out.redacted.contains("Niño"));
        assert!(out.redacted.contains("[REDACTED:NUMERIC_ID]"));
        assert!(out.redacted.contains("ECG normal"));
    }

    #[test]
    fn empty_input_returns_empty_output() {
        let d = PlaceholderDeidentifier::new();
        let out = d.redact("").unwrap();
        assert_eq!(out.redacted, "");
        assert!(out.entities.is_empty());
    }
}
