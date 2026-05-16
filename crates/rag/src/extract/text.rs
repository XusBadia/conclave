//! TXT and Markdown extractors. Both share the same code path — UTF-8 read
//! plus line-ending normalisation.

use std::path::Path;

use conclave_core::{Error, Result};

use super::{normalise_line_endings, DocType, ExtractedText};

pub(super) fn extract(path: &Path, doc_type: DocType) -> Result<ExtractedText> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    Ok(ExtractedText {
        content: normalise_line_endings(raw),
        page_breaks: Vec::new(),
        source_path: path.to_path_buf(),
        doc_type,
        needs_ocr: false,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn reads_utf8_text_file() {
        let mut tmp = tempfile::Builder::new().suffix(".txt").tempfile().unwrap();
        writeln!(tmp, "hola mundo, ¿qué tal?").unwrap();
        let out = extract(tmp.path(), DocType::Txt).unwrap();
        assert!(out.content.contains("hola mundo"));
        assert!(out.content.contains("¿qué tal?"));
        assert_eq!(out.doc_type, DocType::Txt);
        assert!(!out.needs_ocr);
        assert!(out.page_breaks.is_empty());
    }

    #[test]
    fn normalises_crlf_in_markdown() {
        let mut tmp = tempfile::Builder::new().suffix(".md").tempfile().unwrap();
        tmp.write_all(b"# title\r\nbody\r\n").unwrap();
        let out = extract(tmp.path(), DocType::Md).unwrap();
        assert_eq!(out.content, "# title\nbody\n");
        assert_eq!(out.doc_type, DocType::Md);
    }
}
