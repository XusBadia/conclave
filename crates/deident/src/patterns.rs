//! Layer A — regex-driven detectors for high-confidence PII categories.

use std::sync::OnceLock;

use regex::Regex;

use super::{DetectionSource, PiiCategory, RawSpan};

/// Conservative age threshold used by the AGE detector. The pipeline-level
/// config can override the value at run time.
pub(crate) const fn age_threshold() -> u8 {
    89
}

/// Every detector implements this small trait so we can iterate uniformly.
pub(crate) trait Detector: Sync + Send {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>);
}

/// Convenience: get the static list of Layer A detectors.
pub(crate) fn all_detectors() -> &'static [&'static dyn Detector] {
    static DETECTORS: OnceLock<Vec<&'static dyn Detector>> = OnceLock::new();
    DETECTORS.get_or_init(|| {
        vec![
            &DniDetector as &dyn Detector,
            &NieDetector,
            &EmailDetector,
            &PhoneDetector,
            &MrnDetector,
            &DateDetector,
            &AgeDetector,
            &LocationDetector,
        ]
    })
}

// --------------------------------------------------------------------------
// DNI: 8 digits + uppercase letter. We don't validate the check letter; we
// prefer over-masking.
// --------------------------------------------------------------------------
struct DniDetector;
impl Detector for DniDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"\b\d{8}[A-Z]\b").expect("dni regex compiles"));
        for m in re.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Dni,
                confidence: 0.98,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// NIE: [XYZ] + 7 digits + uppercase letter.
// --------------------------------------------------------------------------
struct NieDetector;
impl Detector for NieDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"\b[XYZ]\d{7}[A-Z]\b").expect("nie regex compiles"));
        for m in re.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Nie,
                confidence: 0.98,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// Email — standard match.
// --------------------------------------------------------------------------
struct EmailDetector;
impl Detector for EmailDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")
                .expect("email regex compiles")
        });
        for m in re.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Email,
                confidence: 0.99,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// Phone — international and Spanish formats.
// --------------------------------------------------------------------------
struct PhoneDetector;
impl Detector for PhoneDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        // Match an optional + and country, then 8+ digits with optional
        // separators. Loose on purpose.
        let re = RE.get_or_init(|| {
            Regex::new(
                r"(?x)
                (?:\+\d{1,3}[\s\-]?)?           # optional country code
                \d{2,3}[\s\-]?\d{2,3}[\s\-]?\d{2,4}(?:[\s\-]?\d{2,4})?
                ",
            )
            .expect("phone regex compiles")
        });
        for m in re.find_iter(text) {
            // Require at least 9 digit characters so we don't capture
            // dosages and lab values.
            let digits = m.as_str().chars().filter(char::is_ascii_digit).count();
            if digits < 9 {
                continue;
            }
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Phone,
                confidence: 0.7,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// MRN / NHC / Historia Clínica / Episodio
// --------------------------------------------------------------------------
struct MrnDetector;
impl Detector for MrnDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:NHC|MRN|HC|H\.?C\.?|Historia(?:\s+cl[ií]nica)?|Episodio|Episode)[\s:#\-]*\d{4,}",
            )
            .expect("mrn regex compiles")
        });
        for m in re.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Mrn,
                confidence: 0.95,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// Date — multiple formats, conservative.
// --------------------------------------------------------------------------
struct DateDetector;
impl Detector for DateDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static NUMERIC: OnceLock<Regex> = OnceLock::new();
        let numeric = NUMERIC.get_or_init(|| {
            Regex::new(r"\b(?:\d{1,2}[/\-.]\d{1,2}[/\-.]\d{2,4}|\d{4}[/\-.]\d{1,2}[/\-.]\d{1,2})\b")
                .expect("date numeric regex compiles")
        });
        for m in numeric.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Date,
                confidence: 0.9,
                source: DetectionSource::Rule,
            });
        }

        static SPANISH_LONG: OnceLock<Regex> = OnceLock::new();
        let long = SPANISH_LONG.get_or_init(|| {
            Regex::new(
                r"(?i)\b\d{1,2}\s+de\s+(?:enero|febrero|marzo|abril|mayo|junio|julio|agosto|septiembre|setiembre|octubre|noviembre|diciembre)(?:\s+de\s+\d{4})?",
            )
            .expect("spanish long-date regex compiles")
        });
        for m in long.find_iter(text) {
            out.push(RawSpan {
                start: m.start(),
                end: m.end(),
                category: PiiCategory::Date,
                confidence: 0.95,
                source: DetectionSource::Rule,
            });
        }
    }
}

// --------------------------------------------------------------------------
// Age > threshold (default 89).
// --------------------------------------------------------------------------
struct AgeDetector;
impl Detector for AgeDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r"(?i)\b(\d{1,3})\s*(?:años|anos|year-old|years?\s+old|y\.o\.)\b")
                .expect("age regex compiles")
        });
        for caps in re.captures_iter(text) {
            let Some(num) = caps.get(1) else { continue };
            let Ok(n) = num.as_str().parse::<u8>() else {
                continue;
            };
            if n > age_threshold() {
                let m = caps.get(0).expect("group 0 exists");
                out.push(RawSpan {
                    start: m.start(),
                    end: m.end(),
                    category: PiiCategory::Age,
                    confidence: 0.95,
                    source: DetectionSource::Rule,
                });
            }
        }
    }
}

// --------------------------------------------------------------------------
// Clinical centres and street addresses. These are intentionally conservative
// high-signal patterns so the Location category is useful before a full NER
// layer exists.
// --------------------------------------------------------------------------
struct LocationDetector;
impl Detector for LocationDetector {
    fn detect(&self, text: &str, out: &mut Vec<RawSpan>) {
        collect_location_matches(centre_prefix_re(), 0.86, text, out);
        collect_location_matches(centre_suffix_re(), 0.86, text, out);
        collect_location_matches(spanish_address_re(), 0.88, text, out);
        collect_location_matches(english_address_re(), 0.88, text, out);
    }
}

fn collect_location_matches(re: &Regex, confidence: f32, text: &str, out: &mut Vec<RawSpan>) {
    for m in re.find_iter(text) {
        out.push(RawSpan {
            start: m.start(),
            end: m.end(),
            category: PiiCategory::Location,
            confidence,
            source: DetectionSource::Rule,
        });
    }
}

fn centre_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \b
            (?:
                Hospital|Cl[ií]nica|Centro\s+de\s+Salud|CAP|Ambulatorio|
                Medical\s+Center|Health\s+Center|Clinic
            )
            \s+
            [\p{L}0-9'-]+
            (?:
                \s+
                (?:de|del|la|las|los|the|of|[A-ZÁÉÍÓÚÑ][\p{L}0-9'-]+)
            ){0,5}
            ",
        )
        .expect("centre location regex compiles")
    })
}

fn centre_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \b
            [A-Z][A-Za-z0-9'-]+
            (?:
                \s+
                [A-Z][A-Za-z0-9'-]+
            ){0,5}
            \s+
            (?:Medical\s+Center|Health\s+Center|Clinic|Hospital)
            \b
            ",
        )
        .expect("suffix centre location regex compiles")
    })
}

fn spanish_address_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \b
            (?:C/|Calle|Avenida|Avda\.?|Paseo|Plaza|Camino|Ronda|Rambla)
            \s+
            [\p{L}0-9.'-]+
            (?:
                \s+
                (?:de|del|la|las|los|[A-ZÁÉÍÓÚÑ0-9][\p{L}0-9.'-]*)
            ){0,8}
            (?:,\s*\d+[A-Za-z]?)?
            ",
        )
        .expect("spanish address regex compiles")
    })
}

fn english_address_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?ix)
            \b
            \d{1,5}
            \s+
            [A-Z][A-Za-z0-9.'-]+
            (?:
                \s+
                [A-Z][A-Za-z0-9.'-]+
            ){0,5}
            \s+
            (?:Street|St\.?|Avenue|Ave\.?|Road|Rd\.?|Boulevard|Blvd\.?|Drive|Dr\.?|Lane|Ln\.?)
            \b
            ",
        )
        .expect("english address regex compiles")
    })
}
