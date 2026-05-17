//! PDF extraction with a `pdf-extract` primary and `lopdf` fallback.
//!
//! If both backends fail to produce any text, the document is flagged as
//! `needs_ocr` so the caller can route it to the OCR pipeline (when the
//! `ocr` feature is enabled) or surface a warning to the user.
//!
//! Both `pdf-extract` and `lopdf` are known to panic on certain malformed
//! or unusually-encoded PDFs (CFF font tables, `MinionPro` ligatures, etc.).
//! Every entry point wraps the call in `catch_unwind` so a panic just
//! demotes the document to the next backend (or `needs_ocr`) instead of
//! taking down the Tauri worker / CLI process.

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
    let owned_path = path.to_path_buf();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        pdf_extract::extract_text(&owned_path)
    }));
    match result {
        Ok(Ok(s)) => Some(s),
        Ok(Err(e)) => {
            tracing::debug!(path = %path.display(), error = ?e, "pdf-extract failed");
            None
        }
        Err(panic_payload) => {
            tracing::warn!(
                path = %path.display(),
                panic = %panic_message(&*panic_payload),
                "pdf-extract panicked — falling back to lopdf",
            );
            None
        }
    }
}

fn extract_with_lopdf(path: &Path) -> Option<String> {
    let owned_path = path.to_path_buf();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let doc = lopdf::Document::load(&owned_path).map_err(|e| format!("lopdf load: {e}"))?;
        let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
        if page_numbers.is_empty() {
            return Err("no pages".to_owned());
        }
        doc.extract_text(&page_numbers)
            .map_err(|e| format!("lopdf extract: {e}"))
    }));
    match result {
        Ok(Ok(out)) if !out.trim().is_empty() => Some(out),
        Ok(Ok(_)) => None,
        Ok(Err(e)) => {
            tracing::debug!(path = %path.display(), error = %e, "lopdf failed");
            None
        }
        Err(panic_payload) => {
            tracing::warn!(
                path = %path.display(),
                panic = %panic_message(&*panic_payload),
                "lopdf panicked — marking document as needs_ocr",
            );
            None
        }
    }
}

/// Best-effort extraction of a panic message from a `catch_unwind` payload.
/// `panic!("msg")` and `panic!(format!(…))` yield `&'static str` or `String`;
/// anything else falls through to a placeholder so we never crash on the
/// downcast itself.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic>".to_owned())
}
