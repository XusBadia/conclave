//! HTML → plain-text extraction via `html2text`.

use std::path::Path;

use conclave_core::{Error, Result};

use super::{normalise_line_endings, DocType, ExtractedText};

/// Wrap width passed to `html2text`. Chosen to match what the chunker will
/// later treat as a "natural" paragraph width; not a hard guarantee.
const RENDER_WIDTH: usize = 100;

pub(super) fn extract(path: &Path) -> Result<ExtractedText> {
    let raw = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
    let rendered = html2text::from_read(raw.as_slice(), RENDER_WIDTH)
        .map_err(|e| Error::Rag(format!("html2text: {e}")))?;
    Ok(ExtractedText {
        content: normalise_line_endings(rendered),
        page_breaks: Vec::new(),
        source_path: path.to_path_buf(),
        doc_type: DocType::Html,
        needs_ocr: false,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn strips_tags_and_yields_plain_text() {
        let mut tmp = tempfile::Builder::new().suffix(".html").tempfile().unwrap();
        tmp.write_all(
            "<html><body><h1>Hola</h1><p>Mundo de la cardiología.</p></body></html>".as_bytes(),
        )
        .unwrap();
        let out = extract(tmp.path()).unwrap();
        assert!(out.content.contains("Hola"));
        assert!(out.content.contains("Mundo de la cardiología."));
        assert!(!out.content.contains("<h1>"));
        assert!(!out.content.contains("</p>"));
        assert_eq!(out.doc_type, DocType::Html);
    }
}
