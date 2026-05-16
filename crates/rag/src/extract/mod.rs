//! Document text extraction: PDF, DOCX, TXT, MD, HTML.
//!
//! Dispatches by file extension into per-format submodules. Every extractor
//! returns a normalised [`ExtractedText`] with UTF-8 content and `\n` line
//! endings. PDFs that yield no extractable text are flagged via `needs_ocr`;
//! the OCR backend (`#[cfg(feature = "ocr")]`) handles the rescue path.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

mod docx;
mod html;
mod pdf;
mod text;

#[cfg(feature = "ocr")]
pub mod ocr;

/// Result of running a document through an extractor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedText {
    /// Plain-text content with line endings normalised to `\n`.
    pub content: String,
    /// Byte offsets in `content` where the next page begins. Empty for
    /// formats that lack a page concept (TXT/MD/HTML and the current PDF
    /// extractor pipeline).
    pub page_breaks: Vec<usize>,
    /// Path the bytes were read from.
    pub source_path: PathBuf,
    /// Detected document type.
    pub doc_type: DocType,
    /// `true` when no text could be extracted and OCR is the only viable
    /// path (currently only set for empty PDFs).
    pub needs_ocr: bool,
}

/// File types Conclave knows how to ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    /// Portable Document Format.
    Pdf,
    /// Office Open XML word-processing document.
    Docx,
    /// Plain text.
    Txt,
    /// Markdown.
    Md,
    /// `HyperText` Markup Language.
    Html,
}

impl DocType {
    /// Recognise a document type from a file's extension. Case-insensitive.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "txt" => Some(Self::Txt),
            "md" | "markdown" => Some(Self::Md),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }
}

/// Dispatch text extraction by detecting the document type from `path` and
/// invoking the matching extractor.
pub fn extract_from_path(path: &Path) -> Result<ExtractedText> {
    let doc_type = DocType::from_path(path)
        .ok_or_else(|| Error::Rag(format!("unsupported file type: {}", path.display())))?;
    match doc_type {
        DocType::Pdf => pdf::extract(path),
        DocType::Docx => docx::extract(path),
        DocType::Txt | DocType::Md => text::extract(path, doc_type),
        DocType::Html => html::extract(path),
    }
}

/// Convert `\r\n` and bare `\r` to `\n` without copying when the input is
/// already normalised.
fn normalise_line_endings(s: String) -> String {
    if s.contains('\r') {
        s.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_doc_types_from_extensions() {
        assert_eq!(DocType::from_path(Path::new("a.pdf")), Some(DocType::Pdf));
        assert_eq!(DocType::from_path(Path::new("A.PDF")), Some(DocType::Pdf));
        assert_eq!(DocType::from_path(Path::new("a.docx")), Some(DocType::Docx));
        assert_eq!(DocType::from_path(Path::new("a.txt")), Some(DocType::Txt));
        assert_eq!(DocType::from_path(Path::new("a.md")), Some(DocType::Md));
        assert_eq!(
            DocType::from_path(Path::new("a.markdown")),
            Some(DocType::Md)
        );
        assert_eq!(DocType::from_path(Path::new("a.html")), Some(DocType::Html));
        assert_eq!(DocType::from_path(Path::new("a.htm")), Some(DocType::Html));
        assert_eq!(DocType::from_path(Path::new("a.xyz")), None);
        assert_eq!(DocType::from_path(Path::new("no-ext")), None);
    }

    #[test]
    fn unknown_extension_is_rejected_by_dispatcher() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("file.xyz");
        std::fs::write(&path, b"x").unwrap();
        assert!(extract_from_path(&path).is_err());
    }

    #[test]
    fn line_ending_normalisation() {
        assert_eq!(normalise_line_endings("a\r\nb".into()), "a\nb");
        assert_eq!(normalise_line_endings("a\rb".into()), "a\nb");
        assert_eq!(normalise_line_endings("a\nb".into()), "a\nb");
        assert_eq!(normalise_line_endings(String::new()), "");
    }
}
