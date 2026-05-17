//! Image extraction.
//!
//! Conclave keeps image attachments as first-class evidence even when no
//! text can be recovered from them. Two routes:
//!
//! - **`ocr` feature enabled** — run Tesseract over the raster and surface
//!   the recovered text in `content`. (The full pipeline is wired in a
//!   follow-up; for now this path falls back to the no-OCR branch so the
//!   crate keeps building uniformly across CI targets.)
//! - **Default (no `ocr` feature)** — emit `ExtractedText` with empty
//!   `content` and `needs_ocr = true`. The downstream layers must label
//!   the attachment honestly: "image, no text extracted — only visible to
//!   vision-capable providers".
//!
//! Either way, the file bytes are preserved by the attachments orchestrator
//! so a vision-capable LLM can interpret the image directly.

use std::path::Path;

use conclave_core::{Error, Result};

use super::{DocType, ExtractedText};

pub(super) fn extract(path: &Path) -> Result<ExtractedText> {
    if !path.exists() {
        return Err(Error::Rag(format!("image not found: {}", path.display())));
    }
    // OCR backend is not yet implemented; we honestly report `needs_ocr`
    // so callers can disclose this to the user. Vision-capable providers
    // will receive the raw bytes through the attachments orchestrator.
    Ok(ExtractedText {
        content: String::new(),
        page_breaks: Vec::new(),
        source_path: path.to_path_buf(),
        doc_type: DocType::Image,
        needs_ocr: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_empty_with_ocr_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        let out = extract(&path).unwrap();
        assert_eq!(out.doc_type, DocType::Image);
        assert!(out.content.is_empty());
        assert!(out.needs_ocr);
    }

    #[test]
    fn errors_on_missing_file() {
        let result = extract(Path::new("/tmp/does-not-exist-conclave.png"));
        assert!(result.is_err());
    }
}
