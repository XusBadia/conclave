//! Document loaders.
//!
//! Each loader turns a raw file into a [`Document`] — a normalised plain-text
//! representation plus metadata. Chunking, embedding and storage all operate
//! on the normalised text, never on the source bytes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

pub mod docx;
pub mod html;
pub mod markdown;
pub mod pdf;

/// Source format of a loaded document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    /// Plain `.txt` files.
    PlainText,
    /// Markdown (`.md`, `.markdown`).
    Markdown,
    /// PDF (text-based; scanned PDFs require OCR which is out of scope here).
    Pdf,
    /// HTML / XHTML.
    Html,
    /// DOCX (OOXML word-processing format).
    Docx,
}

impl DocumentFormat {
    /// Infer the format from a path extension.
    ///
    /// Returns `None` for unrecognised extensions; callers decide whether to
    /// skip the file or surface an error.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "txt" | "text" => Some(Self::PlainText),
            "md" | "markdown" => Some(Self::Markdown),
            "pdf" => Some(Self::Pdf),
            "html" | "htm" | "xhtml" => Some(Self::Html),
            "docx" => Some(Self::Docx),
            _ => None,
        }
    }

    /// Short human-readable label used in logs and the CLI.
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlainText => "text",
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
            Self::Html => "html",
            Self::Docx => "docx",
        }
    }
}

/// A normalised document produced by one of the loaders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Absolute path the document was loaded from.
    pub path: PathBuf,
    /// Format the loader identified.
    pub format: DocumentFormat,
    /// Optional title extracted from the document (first heading, file stem,
    /// DOCX core properties, …).
    pub title: Option<String>,
    /// Normalised plain-text contents used by chunking and search.
    pub text: String,
}

impl Document {
    /// Build a new document.
    pub fn new(
        path: impl Into<PathBuf>,
        format: DocumentFormat,
        title: Option<String>,
        text: String,
    ) -> Self {
        Self {
            path: path.into(),
            format,
            title,
            text,
        }
    }
}

/// Load any supported file by inferring the format from its extension.
///
/// # Errors
/// Returns [`Error::Rag`] if the extension is unrecognised, or whatever the
/// per-format loader surfaces (I/O failures, malformed input).
pub fn load_path(path: impl AsRef<Path>) -> Result<Document> {
    let path = path.as_ref();
    let format = DocumentFormat::from_path(path).ok_or_else(|| {
        Error::Rag(format!(
            "unsupported file extension for {path}",
            path = path.display()
        ))
    })?;
    match format {
        DocumentFormat::PlainText => markdown::load_plain_text(path),
        DocumentFormat::Markdown => markdown::load_markdown(path),
        DocumentFormat::Pdf => pdf::load_pdf(path),
        DocumentFormat::Html => html::load_html(path),
        DocumentFormat::Docx => docx::load_docx(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_inference_from_extension() {
        let cases = [
            ("a.txt", Some(DocumentFormat::PlainText)),
            ("a.TXT", Some(DocumentFormat::PlainText)),
            ("a.md", Some(DocumentFormat::Markdown)),
            ("a.markdown", Some(DocumentFormat::Markdown)),
            ("a.pdf", Some(DocumentFormat::Pdf)),
            ("a.html", Some(DocumentFormat::Html)),
            ("a.HTM", Some(DocumentFormat::Html)),
            ("a.docx", Some(DocumentFormat::Docx)),
            ("a.bin", None),
            ("a", None),
        ];
        for (name, expected) in cases {
            assert_eq!(
                DocumentFormat::from_path(Path::new(name)),
                expected,
                "{name}"
            );
        }
    }
}
