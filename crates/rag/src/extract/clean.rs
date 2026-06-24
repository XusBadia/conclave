//! Conservative boilerplate stripping for ingested documents.
//!
//! Guideline PDFs carry running headers/footers, bare page numbers,
//! copyright/ISBN lines and table-of-contents entries that pollute
//! retrieval — the model ends up citing front-matter instead of clinical
//! recommendations. We drop the safe-to-remove classes BEFORE chunking so
//! `[E*]` evidence never carries pure boilerplate.
//!
//! The pass is deliberately conservative: it only removes lines that match
//! narrow, high-precision patterns, or that repeat verbatim across many pages
//! (running heads/feet). When in doubt it keeps the line, and if stripping
//! would empty an otherwise non-empty document it returns the original text
//! untouched.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

/// Minimum number of verbatim repetitions for a short line to be treated as a
/// running header/footer. Clinical recommendations do not repeat 4+ times.
const RUNNING_HEAD_MIN_REPEATS: usize = 4;

/// A line must be at most this many characters to qualify as a running head.
const RUNNING_HEAD_MAX_CHARS: usize = 80;

/// Strip common boilerplate from extracted document text, line by line.
///
/// Returns the cleaned text, or the original `content` unchanged if stripping
/// would remove everything (a guard against over-deletion).
#[must_use]
pub fn strip_boilerplate(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    // Count verbatim (trimmed) non-empty lines to spot running heads/feet,
    // which repeat across page boundaries.
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for line in &lines {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            *freq.entry(trimmed).or_insert(0) += 1;
        }
    }

    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            kept.push(line);
            continue;
        }
        let repeats = freq.get(trimmed).copied().unwrap_or(0);
        if is_boilerplate(trimmed, repeats) {
            continue;
        }
        kept.push(line);
    }

    let cleaned = kept.join("\n");
    if cleaned.trim().is_empty() {
        content.to_owned()
    } else {
        cleaned
    }
}

/// Whether a single trimmed line is safe-to-drop boilerplate.
fn is_boilerplate(line: &str, repeats: usize) -> bool {
    page_number_re().is_match(line)
        || toc_leader_re().is_match(line)
        || legal_re().is_match(line)
        || url_only_re().is_match(line)
        || is_running_head(line, repeats)
}

/// Frequent short lines that repeat across pages are running heads/feet.
fn is_running_head(line: &str, repeats: usize) -> bool {
    repeats >= RUNNING_HEAD_MIN_REPEATS && line.chars().count() <= RUNNING_HEAD_MAX_CHARS
}

/// Bare page numbers: `12`, `- 12 -`, `Page 12`, `Página 12`, `pág. 7`, `p. 3`.
fn page_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^[\s\-–—|]*(?:p(?:ag(?:e|ina|\.)?|ág(?:ina|\.)?|\.)\s*)?\d{1,4}[\s\-–—|.]*$",
        )
        .expect("hand-written regex must compile")
    })
}

/// Table-of-contents dotted leaders: `Introducción .......... 12`. Matches a
/// run of 3+ literal dots or 2+ ellipsis glyphs (a single prose `…` is kept).
fn toc_leader_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:\.{3,}|\u{2026}{2,})\s*\d{1,4}\s*$")
            .expect("hand-written regex must compile")
    })
}

/// Copyright / rights / ISBN / e-ISSN / DOI lines.
fn legal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(©|\(c\)\s*\d{4}|\bcopyright\b|all rights reserved|todos los derechos reservados|\bisbn\b|\be-?issn\b|\bdoi:\s*\S)",
        )
        .expect("hand-written regex must compile")
    })
}

/// A whole line that is just a URL or a bare domain.
fn url_only_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:https?://|www\.)\S+\s*$").expect("hand-written regex must compile")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_clinical_recommendations() {
        let line = "En estadio pT2N0 se recomienda seguimiento sin tratamiento adyuvante.";
        assert!(!is_boilerplate(line, 1), "recommendation must be kept");
    }

    #[test]
    fn drops_page_numbers() {
        for line in ["12", "- 12 -", "Page 12", "Página 12", "pág. 7", "p. 3"] {
            assert!(
                is_boilerplate(line, 1),
                "expected page number dropped: {line:?}"
            );
        }
        // A dosage line that merely starts with a number must survive.
        assert!(!is_boilerplate("5 mg de furosemida intravenosa", 1));
    }

    #[test]
    fn drops_toc_leaders() {
        assert!(is_boilerplate("Introducción .......... 12", 1));
        assert!(is_boilerplate("Estadificación TNM…… 34", 1));
        // A single abbreviated dot is not a leader.
        assert!(!is_boilerplate("Véase la Fig. 3 para el algoritmo", 1));
    }

    #[test]
    fn drops_legal_and_url_lines() {
        for line in [
            "© 2021 Sociedad Española de Oncología",
            "Copyright belongs to the publisher",
            "ISBN 978-84-1234-567-8",
            "https://example.org/guideline.pdf",
            "www.oncoguia.es",
        ] {
            assert!(
                is_boilerplate(line, 1),
                "expected legal/url dropped: {line:?}"
            );
        }
    }

    #[test]
    fn drops_repeated_running_heads_only_when_frequent() {
        let head = "Guía de Práctica Clínica · Cáncer Colorrectal";
        assert!(is_boilerplate(head, RUNNING_HEAD_MIN_REPEATS));
        assert!(
            !is_boilerplate(head, 2),
            "two repeats is not enough to drop"
        );
    }

    #[test]
    fn end_to_end_strips_boilerplate_keeps_body() {
        let raw = "Guía Clínica\n\
                   Guía Clínica\n\
                   Guía Clínica\n\
                   Guía Clínica\n\
                   1\n\
                   Índice .......... 3\n\
                   © 2021 Editorial\n\
                   La resección anterior baja es el tratamiento de elección.\n\
                   2";
        let cleaned = strip_boilerplate(raw);
        assert!(cleaned.contains("resección anterior baja"));
        assert!(!cleaned.contains("Índice"));
        assert!(!cleaned.contains("©"));
        assert!(!cleaned.contains("Guía Clínica"));
    }

    #[test]
    fn over_deletion_guard_returns_original() {
        // A document that is ALL boilerplate would otherwise become empty;
        // the guard returns the original so we never lose a document.
        let raw = "1\n2\n3\n© 2021";
        assert_eq!(strip_boilerplate(raw), raw);
    }
}
