//! PDF extraction with a `pdf-extract` primary and `lopdf` fallback.
//!
//! If both backends fail to produce any text, the document is flagged as
//! `needs_ocr` so the caller can route it to the OCR pipeline (when the
//! `ocr` feature is enabled) or surface a warning to the user.

use std::path::Path;

use conclave_core::Result;

use super::{normalise_line_endings, DocType, ExtractedText};

pub(super) fn extract(path: &Path) -> Result<ExtractedText> {
    let content = extract_with_pdf_extract(path)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| extract_with_lopdf(path).filter(|s| !s.trim().is_empty()))
        .unwrap_or_default();

    let needs_ocr = content.trim().is_empty();
    Ok(ExtractedText {
        content: normalise_line_endings(content),
        page_breaks: Vec::new(),
        source_path: path.to_path_buf(),
        doc_type: DocType::Pdf,
        needs_ocr,
    })
}

fn extract_with_pdf_extract(path: &Path) -> Option<String> {
    match pdf_extract::extract_text(path) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::debug!(path = %path.display(), error = ?e, "pdf-extract failed");
            None
        }
    }
}

fn extract_with_lopdf(path: &Path) -> Option<String> {
    let doc = match lopdf::Document::load(path) {
        Ok(doc) => doc,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = ?e, "lopdf load failed");
            return None;
        }
    };
    let mut out = String::new();
    let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    if page_numbers.is_empty() {
        return None;
    }
    match doc.extract_text(&page_numbers) {
        Ok(text) => out.push_str(&text),
        Err(e) => {
            tracing::debug!(path = %path.display(), error = ?e, "lopdf extract failed");
            return None;
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}
