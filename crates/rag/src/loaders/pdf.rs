//! PDF loader (text-based PDFs only; scanned PDFs need OCR, out of scope).

use std::path::Path;

use conclave_core::{Error, Result};

use super::{Document, DocumentFormat};

/// Load a PDF file and extract its plain-text content.
///
/// # Errors
/// Returns [`Error::Rag`] if `pdf-extract` cannot parse the file (encrypted,
/// malformed, or image-only scans without an embedded text layer).
pub fn load_pdf(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
    let raw = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| Error::Rag(format!("pdf extraction failed for {}: {e}", path.display())))?;
    let text = normalize_pdf_text(&raw);
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace(['_', '-'], " "))
        .filter(|s| !s.trim().is_empty());
    Ok(Document::new(path, DocumentFormat::Pdf, title, text))
}

/// Collapse the runaway whitespace and page-break form-feeds that
/// `pdf-extract` leaves behind into something a tokenizer can consume.
fn normalize_pdf_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_blank = true;
    for line in raw.lines() {
        let trimmed = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{000C}');
        if trimmed.is_empty() {
            if !prev_blank {
                out.push('\n');
                prev_blank = true;
            }
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(trimmed);
        prev_blank = false;
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_blank_runs_and_form_feeds() {
        let raw = "Page 1 header\n\n\nBody text\n\u{000C}Page 2\n\n\nMore body\n";
        let out = normalize_pdf_text(raw);
        assert_eq!(out, "Page 1 header\nBody text\nPage 2\nMore body");
    }
}
