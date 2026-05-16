//! OCR rescue path for scanned PDFs (gated by the `ocr` Cargo feature).
//!
//! When enabled, this module rasterises each PDF page to 300 DPI with
//! `pdfium-render` and runs `tesseract-rs` with `spa+eng` to recover text.
//! The result is wrapped in an [`ExtractedText`] just like the regular PDF
//! extractor.
//!
//! The full implementation lands in a Phase 1 follow-up — landing the
//! feature gate and module skeleton first lets CI exercise the `--features
//! ocr` build on Linux without blocking on a working pipeline.

use std::path::{Path, PathBuf};

use conclave_core::{Error, Result};

use super::{DocType, ExtractedText};

/// Run OCR on a PDF whose text-layer extraction returned empty.
///
/// # Errors
/// Always returns [`Error::Rag`] for now; the real implementation will be
/// wired in once we have a scanned-PDF fixture and the binding layout is
/// stable on all CI targets.
pub fn ocr_pdf(path: &Path) -> Result<ExtractedText> {
    let source_path: PathBuf = path.to_path_buf();
    // Touch the source path so the unused-variable lint stays quiet and the
    // intent of the signature is preserved when the body lands.
    tracing::warn!(path = %source_path.display(), "ocr_pdf called but not yet implemented");
    Err(Error::Rag(
        "OCR pipeline is wired but not yet implemented — Phase 1 follow-up".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_rag_error() {
        let result = ocr_pdf(Path::new("does-not-exist.pdf"));
        assert!(matches!(result, Err(Error::Rag(_))));
    }

    #[test]
    fn doc_type_pdf_is_what_we_target() {
        // Sanity: ensure the public DocType still exposes Pdf so the
        // future implementation can build an ExtractedText with it.
        let _ = DocType::Pdf;
    }
}
