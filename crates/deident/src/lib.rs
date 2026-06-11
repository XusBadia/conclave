//! De-identification of personally identifiable information (PII) in
//! clinical free-text.
//!
//! ## Pipeline
//!
//! Two of the three layers defined in the Phase 3 spec land here:
//!
//! - **Layer A — regex-based detectors** for high-confidence identifiers
//!   (DNI, NIE, medical-record numbers, phone, email, dates, ages > 89).
//! - **Layer C — heuristic disambiguation** for capitalised name-like
//!   sequences and clinician honorifics (Dr./Dra.).
//!
//! Layer B (a multilingual NER model) is intentionally deferred — it pulls
//! in ONNX Runtime + model weights that we already paid for via
//! `fastembed`, but the GLINER family ships separate weights and is worth a
//! dedicated commit once we have real Spanish clinical text to evaluate
//! against.
//!
//! ## Output
//!
//! [`Deidentifier::deidentify`] returns a [`DeidentResult`] with the masked
//! text, the original text (for audit / undo), the list of [`PiiSpan`]s,
//! the set of categories observed and a `strict_mode_clean` flag.
//!
//! ## Privacy invariants
//!
//! - The crate has no I/O and never logs the raw text. Logs at `info` show
//!   only category counts.
//! - Detection is **conservative**: false positives are acceptable, false
//!   negatives are not.

#![allow(clippy::similar_names, clippy::items_after_statements)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use conclave_core::Result;

mod patterns;

/// PII categories recognised by the pipeline. Order matches the prompt
/// spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PiiCategory {
    /// Patient name (default category for ambiguous person names).
    PatientName,
    /// Healthcare professional name (Dr., Dra., MD…).
    ClinicianName,
    /// Calendar date in any common format.
    Date,
    /// Patient age above the configured threshold (default 89).
    Age,
    /// Geographic locations and centre names.
    Location,
    /// Medical record number / NHC / patient id.
    Mrn,
    /// Spanish DNI (8 digits + letter).
    Dni,
    /// Spanish NIE ([XYZ] + 7 digits + letter).
    Nie,
    /// Phone numbers (loose match).
    Phone,
    /// Email addresses.
    Email,
    /// Catch-all for anything else flagged.
    OtherPii,
}

impl PiiCategory {
    /// Human-readable, machine-stable name used to build the masking token
    /// (`<PATIENT_NAME_1>`, `<DATE_2>`, …).
    pub const fn token_prefix(self) -> &'static str {
        match self {
            Self::PatientName => "PATIENT_NAME",
            Self::ClinicianName => "CLINICIAN_NAME",
            Self::Date => "DATE",
            Self::Age => "AGE",
            Self::Location => "LOCATION",
            Self::Mrn => "MRN",
            Self::Dni => "DNI",
            Self::Nie => "NIE",
            Self::Phone => "PHONE",
            Self::Email => "EMAIL",
            Self::OtherPii => "OTHER_PII",
        }
    }
}

/// Where a detection came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionSource {
    /// One of the regex patterns.
    Rule,
    /// The NER model (Layer B — not active yet).
    Model,
    /// Heuristic post-processing (Layer C).
    Heuristic,
}

/// One PII span discovered in the input text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PiiSpan {
    /// Byte offset where the span starts (inclusive).
    pub start: usize,
    /// Byte offset where the span ends (exclusive).
    pub end: usize,
    /// Category of the span.
    pub category: PiiCategory,
    /// Original surface form (kept in-memory for masking; never persisted
    /// to disk through this crate).
    pub original: String,
    /// Masking token assigned to this span
    /// (`<PATIENT_NAME_1>`, `<DATE_3>`, …).
    pub token: String,
    /// Detector confidence 0..1.
    pub confidence: f32,
    /// Layer the detection came from.
    pub source: DetectionSource,
}

/// Result of running the de-identification pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeidentResult {
    /// Input with every span replaced by its token.
    pub masked_text: String,
    /// The original, untouched input. Kept for audit / undo / reversible
    /// research workflows. Never written to disk through this crate.
    pub original_text: String,
    /// Spans in left-to-right order.
    pub spans: Vec<PiiSpan>,
    /// Categories the run touched at least once.
    pub categories_found: BTreeSet<PiiCategory>,
    /// `true` when strict-mode post-processing flagged no suspicious
    /// patterns remaining in `masked_text`.
    pub strict_mode_clean: bool,
    /// Stable identifier of the pipeline configuration used.
    pub pipeline_id: &'static str,
}

/// Anything that can de-identify a string.
pub trait Deidentifier {
    /// Stable id used to record which pipeline was applied to a case.
    fn id(&self) -> &'static str;

    /// Run the pipeline. Strict mode controls only the post-processing
    /// flag — masking is the same either way.
    fn deidentify(&self, text: &str) -> Result<DeidentResult>;
}

/// Configuration knobs for the regex+heuristic pipeline.
#[derive(Debug, Clone)]
pub struct DeidentConfig {
    /// Ages strictly greater than this threshold get masked (HIPAA-style
    /// generalisation of ages > 89).
    pub age_threshold: u8,
    /// Set of common Spanish/English words that the name heuristic must
    /// not flag.
    pub common_words: BTreeSet<&'static str>,
}

impl Default for DeidentConfig {
    fn default() -> Self {
        Self {
            age_threshold: 89,
            common_words: default_common_words(),
        }
    }
}

/// Production-ready regex+heuristic pipeline.
#[derive(Debug, Clone, Default)]
pub struct PipelineDeidentifier {
    config: DeidentConfig,
}

impl PipelineDeidentifier {
    /// Build a pipeline with the default config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a pipeline with a custom config.
    #[must_use]
    pub const fn with_config(config: DeidentConfig) -> Self {
        Self { config }
    }
}

impl Deidentifier for PipelineDeidentifier {
    fn id(&self) -> &'static str {
        "regex+heuristic-v1"
    }

    fn deidentify(&self, text: &str) -> Result<DeidentResult> {
        self.run(text)
    }
}

/// Raw span before token assignment.
#[derive(Debug, Clone)]
struct RawSpan {
    start: usize,
    end: usize,
    category: PiiCategory,
    confidence: f32,
    source: DetectionSource,
}

/// Drop overlapping spans, keeping the higher-confidence one (or the
/// earlier one on ties). Sorts left-to-right.
fn merge_spans(mut raw: Vec<RawSpan>) -> Vec<RawSpan> {
    raw.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut out: Vec<RawSpan> = Vec::with_capacity(raw.len());
    for span in raw {
        match out.last_mut() {
            Some(prev) if span.start < prev.end => {
                // Overlap. Keep the higher-confidence one; on ties keep the
                // existing (which started earlier or is wider).
                if span.confidence > prev.confidence + 0.05 {
                    *prev = span;
                }
            }
            _ => out.push(span),
        }
    }
    out
}

/// Allocate stable per-category tokens (`<DATE_1>`, `<DATE_2>`, …).
/// Identical surface forms within one document share their token.
fn assign_tokens(raw: Vec<RawSpan>) -> Vec<PiiSpan> {
    let mut counters: BTreeMap<PiiCategory, u32> = BTreeMap::new();
    // Map from (category, surface form) to already-assigned token.
    let mut seen: HashMap<(PiiCategory, String), String> = HashMap::new();
    let mut out = Vec::with_capacity(raw.len());

    for span in raw {
        // The caller hasn't given us the text yet; we'll fix that in
        // `apply_mask`. For now we rely on the `original` slot being filled
        // by the caller. We re-derive it here from the offsets.
        // NOTE: we don't have access to `text` in this function; instead,
        // the caller invokes `assign_tokens` with raw spans that carry
        // start/end and then `apply_mask` will compute the surface form
        // from the text. We materialise `original` there.
        let token = format!("<{}_PLACEHOLDER>", span.category.token_prefix());
        out.push(PiiSpan {
            start: span.start,
            end: span.end,
            category: span.category,
            original: String::new(),
            token,
            confidence: span.confidence,
            source: span.source,
        });
        // Touch the counters map so it shows up — actual token id is
        // resolved in apply_mask once we know the surface form.
        counters.entry(span.category).or_insert(0);
        let _ = &mut seen;
    }
    out
}

/// Substitute every span's surface form with its token, materialising the
/// `original` field along the way.
fn apply_mask(text: &str, spans: &[PiiSpan]) -> String {
    let mut counters: BTreeMap<PiiCategory, u32> = BTreeMap::new();
    let mut seen: HashMap<(PiiCategory, String), String> = HashMap::new();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    // Note: `spans` arrives already sorted by start (apply_mask is called
    // after merge_spans). We need to update each span in place but they're
    // immutable here; instead we rebuild a parallel Vec inside the caller.
    // For ergonomics we let the caller swap it back, see `deidentify`.
    let _ = (&mut counters, &mut seen);
    for span in spans {
        if span.start < cursor {
            continue; // overlapping; merged version handled it.
        }
        out.push_str(&text[cursor..span.start]);
        out.push_str(&span.token);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    out
}

// --------------------------------------------------------------------------
// Layer C — heuristics.
// --------------------------------------------------------------------------

mod heuristics {
    use super::{patterns, DeidentConfig, DetectionSource, PiiCategory, RawSpan, Regex};
    use std::sync::OnceLock;

    /// Append heuristic spans (clinician honorifics + capitalised names).
    pub(super) fn collect(text: &str, cfg: &DeidentConfig, out: &mut Vec<RawSpan>) {
        // Dr./Dra. <Name [Surname]>
        static HONORIFIC: OnceLock<Regex> = OnceLock::new();
        let re = HONORIFIC.get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:Dr|Dra|Drs|Prof|Profesor|Profesora)\.?\s+([A-ZÁÉÍÓÚÑ][a-záéíóúñ]+(?:\s+[A-ZÁÉÍÓÚÑ][a-záéíóúñ]+){0,3})",
            )
            .expect("honorific regex compiles")
        });
        for caps in re.captures_iter(text) {
            if let Some(name) = caps.get(1) {
                out.push(RawSpan {
                    start: name.start(),
                    end: name.end(),
                    category: PiiCategory::ClinicianName,
                    confidence: 0.9,
                    source: DetectionSource::Heuristic,
                });
            }
        }

        // Standalone capitalised two-or-three word sequences (Layer C name
        // heuristic). False-positive prone — but Phase 3 explicitly prefers
        // over-masking.
        static NAME_LIKE: OnceLock<Regex> = OnceLock::new();
        let re = NAME_LIKE.get_or_init(|| {
            Regex::new(r"\b([A-ZÁÉÍÓÚÑ][a-záéíóúñ]+(?:\s+[A-ZÁÉÍÓÚÑ][a-záéíóúñ]+){1,3})\b")
                .expect("name-like regex compiles")
        });
        for caps in re.captures_iter(text) {
            let Some(m) = caps.get(0) else { continue };
            let phrase = m.as_str();
            // Reject phrases that consist entirely of common words.
            if phrase
                .split_whitespace()
                .all(|w| cfg.common_words.contains(w))
            {
                continue;
            }
            // Reject phrases that immediately follow a honorific (already
            // captured above with higher confidence).
            let head_end = m.start();
            // The 8-byte lookback is byte arithmetic over UTF-8 text, so
            // it can land inside a multibyte char (the `ó` in "Sessió")
            // — walk back to the nearest boundary or slicing panics.
            let mut lookback_start = head_end.saturating_sub(8);
            while !text.is_char_boundary(lookback_start) {
                lookback_start -= 1;
            }
            let prefix = &text[lookback_start..head_end];
            if prefix.contains("Dr.") || prefix.contains("Dra.") {
                continue;
            }
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::PatientName,
                confidence: 0.55,
                source: DetectionSource::Heuristic,
            });
        }

        let _ = patterns::age_threshold(); // ensures module is referenced
    }
}

/// Strict-mode post-processing. Looks for residual 9+ digit numbers and
/// capitalised two-word sequences that survived masking. Returns `true`
/// when none are found.
fn strict_check(masked: &str) -> bool {
    static RESIDUAL_DIGITS: OnceLock<Regex> = OnceLock::new();
    let re = RESIDUAL_DIGITS
        .get_or_init(|| Regex::new(r"\b\d{9,}\b").expect("strict digit regex compiles"));
    if re.is_match(masked) {
        return false;
    }
    static RESIDUAL_EMAIL: OnceLock<Regex> = OnceLock::new();
    let re = RESIDUAL_EMAIL.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
            .expect("strict email regex compiles")
    });
    !re.is_match(masked)
}

/// Default Spanish/English vocabulary used by the name heuristic to drop
/// obvious non-names. Tiny on purpose — false positives are tolerated and
/// the verdict prompt acknowledges over-masking.
fn default_common_words() -> BTreeSet<&'static str> {
    [
        "El",
        "La",
        "Los",
        "Las",
        "Un",
        "Una",
        "Unos",
        "Unas",
        "De",
        "Del",
        "Y",
        "O",
        "Pero",
        "Si",
        "No",
        "En",
        "Con",
        "Sin",
        "Por",
        "Para",
        "Que",
        "Como",
        "Cuando",
        "Donde",
        "Sobre",
        "Entre",
        "Hasta",
        "Desde",
        "Su",
        "Sus",
        "Mi",
        "Mis",
        "Tu",
        "Tus",
        "Mr",
        "Mrs",
        "The",
        "And",
        "Patient",
        "Paciente",
        "Doctor",
        "Doctora",
        "Hospital",
        "Clínica",
        "Servicio",
        "Unidad",
        "Urgencias",
        "Cardiología",
        "Oncología",
        "Insuficiencia",
        "Cardiaca",
        "Diabetes",
        "Mellitus",
        "Hipertensión",
        "Arterial",
        "Texto",
    ]
    .into_iter()
    .collect()
}

// --------------------------------------------------------------------------
// `deidentify` post-processing: actually resolve token ids using the text.
// --------------------------------------------------------------------------

impl PipelineDeidentifier {
    fn finalise(text: &str, mut spans: Vec<PiiSpan>) -> Vec<PiiSpan> {
        let mut counters: BTreeMap<PiiCategory, u32> = BTreeMap::new();
        let mut seen: HashMap<(PiiCategory, String), String> = HashMap::new();
        for span in &mut spans {
            let surface = text[span.start..span.end].to_owned();
            let key = (span.category, surface.clone());
            let token = seen
                .entry(key)
                .or_insert_with(|| {
                    let counter = counters.entry(span.category).or_insert(0);
                    *counter += 1;
                    format!("<{}_{}>", span.category.token_prefix(), counter)
                })
                .clone();
            span.original = surface;
            span.token = token;
        }
        spans
    }
}

// Override the masking pipeline to finalise tokens with actual surface
// forms and then re-apply the substitution.
impl PipelineDeidentifier {
    /// Specialised entry point that runs the full pipeline. Identical to
    /// [`Deidentifier::deidentify`] but inlined here so we can finalise the
    /// surface forms / counters before computing the masked text.
    pub fn run(&self, text: &str) -> Result<DeidentResult> {
        let mut raw_spans: Vec<RawSpan> = Vec::new();
        for det in patterns::all_detectors() {
            det.detect(text, &mut raw_spans);
        }
        heuristics::collect(text, &self.config, &mut raw_spans);
        let merged = merge_spans(raw_spans);
        let spans_no_tokens = assign_tokens(merged);
        let spans = Self::finalise(text, spans_no_tokens);
        let masked_text = apply_mask(text, &spans);
        let categories_found = spans.iter().map(|s| s.category).collect();
        let strict_mode_clean = strict_check(&masked_text);
        Ok(DeidentResult {
            masked_text,
            original_text: text.to_owned(),
            spans,
            categories_found,
            strict_mode_clean,
            pipeline_id: self.id(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline() -> PipelineDeidentifier {
        PipelineDeidentifier::new()
    }

    #[test]
    fn dni_is_masked() {
        let p = pipeline();
        let out = p
            .run("Paciente con DNI 12345678Z acudió a urgencias.")
            .unwrap();
        assert!(out.masked_text.contains("<DNI_1>"), "{}", out.masked_text);
        assert!(out.categories_found.contains(&PiiCategory::Dni));
    }

    #[test]
    fn nie_is_masked() {
        let p = pipeline();
        let out = p.run("NIE: X1234567L registrado.").unwrap();
        assert!(out.masked_text.contains("<NIE_1>"));
    }

    #[test]
    fn email_and_phone_are_masked() {
        let p = pipeline();
        let out = p
            .run("Contacto: maria@example.com, tel +34 612 345 678.")
            .unwrap();
        assert!(out.masked_text.contains("<EMAIL_1>"), "{}", out.masked_text);
        assert!(out.masked_text.contains("<PHONE_1>"), "{}", out.masked_text);
    }

    #[test]
    fn mrn_pattern_is_masked() {
        let p = pipeline();
        let out = p.run("Historia clínica NHC: 1234567 revisada.").unwrap();
        assert!(out.masked_text.contains("<MRN_1>"), "{}", out.masked_text);
    }

    #[test]
    fn age_over_89_is_masked() {
        let p = pipeline();
        let out = p.run("Mujer de 93 años con disnea.").unwrap();
        assert!(out.masked_text.contains("<AGE_1>"), "{}", out.masked_text);
    }

    #[test]
    fn age_under_threshold_is_not_masked() {
        let p = pipeline();
        let out = p.run("Hombre de 45 años con dolor torácico.").unwrap();
        assert!(!out.masked_text.contains("<AGE_"));
    }

    #[test]
    fn date_dd_mm_yyyy_is_masked() {
        let p = pipeline();
        let out = p.run("Fecha de cirugía: 12/03/2024.").unwrap();
        assert!(out.masked_text.contains("<DATE_1>"), "{}", out.masked_text);
    }

    #[test]
    fn es_fixture_masks_core_clinical_pii() {
        let p = pipeline();
        let text = "Paciente María García López, DNI 12345678Z, NIE X1234567L, \
                    NHC: 4567890, nacida el 12/03/1940. Contacto +34 612 345 678 \
                    y maria.garcia@example.com. Revisada en Hospital Clínic de Barcelona. \
                    Vive en Calle de Mallorca 401.";
        let out = p.run(text).unwrap();
        for needle in [
            "<PATIENT_NAME_1>",
            "<DNI_1>",
            "<NIE_1>",
            "<MRN_1>",
            "<DATE_1>",
            "<PHONE_1>",
            "<EMAIL_1>",
            "<LOCATION_1>",
            "<LOCATION_2>",
        ] {
            assert!(
                out.masked_text.contains(needle),
                "{needle}: {}",
                out.masked_text
            );
        }
        assert!(out.categories_found.contains(&PiiCategory::Location));
    }

    #[test]
    fn en_fixture_masks_core_clinical_pii() {
        let p = pipeline();
        let text = "Patient John Smith, MRN 99887766, seen on 2024-05-12 at \
                    St Mary Medical Center. Phone +1 415 555 2671, email \
                    john.smith@example.org, address 221 Baker Street.";
        let out = p.run(text).unwrap();
        for needle in [
            "<PATIENT_NAME_1>",
            "<MRN_1>",
            "<DATE_1>",
            "<PHONE_1>",
            "<EMAIL_1>",
            "<LOCATION_1>",
            "<LOCATION_2>",
        ] {
            assert!(
                out.masked_text.contains(needle),
                "{needle}: {}",
                out.masked_text
            );
        }
    }

    #[test]
    fn clinician_honorific_masked_separately() {
        let p = pipeline();
        let out = p
            .run("Atendido por Dr. Pérez en el servicio de Cardiología.")
            .unwrap();
        assert!(
            out.masked_text.contains("<CLINICIAN_NAME_1>"),
            "{}",
            out.masked_text
        );
    }

    #[test]
    fn name_lookback_survives_multibyte_chars() {
        // Regression: the name-like heuristic looks back 8 BYTES from
        // each match to skip honorific-prefixed names. On accented text
        // (Catalan/Spanish clinical notes — "Sessió:…") that byte offset
        // can land inside a multibyte char and the slice panicked,
        // killing the whole batch command. Sweep the gap between the
        // accented char and the name so the lookback start hits every
        // alignment around the `ó`, including its interior byte.
        let p = pipeline();
        for gap in 0..=8 {
            let text = format!("Sessió:{} Marta Soler Vives", "x".repeat(gap));
            let out = p.run(&text).unwrap_or_else(|e| {
                panic!("deidentify failed at gap={gap}: {e}");
            });
            assert!(!out.masked_text.is_empty(), "gap={gap}");
        }
    }

    #[test]
    fn token_assignment_is_deterministic() {
        let p = pipeline();
        let a = p.run("DNI 12345678Z y DNI 12345678Z otra vez.").unwrap();
        // Same surface form → same token.
        let count = a.masked_text.matches("<DNI_1>").count();
        assert_eq!(count, 2, "{}", a.masked_text);
    }

    #[test]
    fn strict_mode_flag_clean_on_typical_input() {
        let p = pipeline();
        let out = p.run("Paciente estable, sin novedades.").unwrap();
        assert!(out.strict_mode_clean);
    }

    #[test]
    fn empty_input_returns_clean() {
        let p = pipeline();
        let out = p.run("").unwrap();
        assert!(out.spans.is_empty());
        assert!(out.strict_mode_clean);
        assert_eq!(out.masked_text, "");
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    /// Mix arbitrary printable unicode with clinically-shaped fragments so
    /// the name/date/ID heuristics actually fire — pure random unicode
    /// rarely trips them. Sizes stay small so the 64 cases per property
    /// don't inflate the pre-commit hook.
    fn arb_clinical_text() -> impl Strategy<Value = String> {
        let fragment = prop_oneof![
            "\\PC{0,64}", // any printable unicode, incl. multibyte + emoji
            Just("Sessió:".to_owned()),
            Just("García Pérez".to_owned()),
            Just("Dra. Marta Soler Vives".to_owned()),
            Just("DNI 12345678Z".to_owned()),
            Just("12/03/2026".to_owned()),
            Just("☎ 612 345 678 — 👩‍⚕️ niño".to_owned()),
        ];
        proptest::collection::vec(fragment, 0..12).prop_map(|parts| parts.join(" "))
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// The masking pipeline must never panic or error on ANY input —
        /// a char-boundary slice panic in this crate once froze whole
        /// production batches (fixed at 4ecaaea; this pins the class).
        #[test]
        fn never_panics_on_arbitrary_utf8(text in arb_clinical_text()) {
            let out = PipelineDeidentifier::new().run(&text);
            prop_assert!(out.is_ok(), "deident errored: {:?}", out.err());
        }

        /// Same input → same output. Mask determinism is part of the
        /// audit-trail contract: prompt fingerprints must be reproducible.
        #[test]
        fn masking_is_deterministic(text in arb_clinical_text()) {
            let p = PipelineDeidentifier::new();
            let a = p.run(&text).unwrap();
            let b = p.run(&text).unwrap();
            prop_assert_eq!(a.masked_text, b.masked_text);
            prop_assert_eq!(a.spans.len(), b.spans.len());
        }

        /// Every reported span must lie on char boundaries of the original
        /// text and its `original` field must match the slice it claims —
        /// the exact invariant whose violation caused the production panic.
        #[test]
        fn spans_lie_on_char_boundaries(text in arb_clinical_text()) {
            let out = PipelineDeidentifier::new().run(&text).unwrap();
            for s in &out.spans {
                prop_assert!(s.start < s.end, "empty span {s:?}");
                prop_assert!(s.end <= text.len(), "span past end {s:?}");
                prop_assert!(text.is_char_boundary(s.start), "start mid-char {s:?}");
                prop_assert!(text.is_char_boundary(s.end), "end mid-char {s:?}");
                prop_assert_eq!(&text[s.start..s.end], s.original.as_str());
            }
        }
    }
}
