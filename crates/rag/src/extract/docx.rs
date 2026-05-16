//! DOCX text extraction via `docx-rs`.
//!
//! The crate is primarily a writer but exposes [`docx_rs::read_docx`] for the
//! read path. We walk the document's paragraph tree and concatenate every
//! `Text` run, emitting a newline between paragraphs. Tables and headers are
//! not extracted yet — they will be wired in a follow-up if real corpora need
//! them.

use std::path::Path;

use conclave_core::{Error, Result};
use docx_rs::{DocumentChild, ParagraphChild, RunChild};

use super::{normalise_line_endings, DocType, ExtractedText};

pub(super) fn extract(path: &Path) -> Result<ExtractedText> {
    let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
    let docx = docx_rs::read_docx(&bytes).map_err(|e| Error::Rag(format!("docx parse: {e}")))?;

    let mut content = String::new();
    for child in &docx.document.children {
        if let DocumentChild::Paragraph(paragraph) = child {
            for p_child in &paragraph.children {
                if let ParagraphChild::Run(run) = p_child {
                    for r_child in &run.children {
                        if let RunChild::Text(t) = r_child {
                            content.push_str(&t.text);
                        }
                    }
                }
            }
            content.push('\n');
        }
    }

    Ok(ExtractedText {
        content: normalise_line_endings(content),
        page_breaks: Vec::new(),
        source_path: path.to_path_buf(),
        doc_type: DocType::Docx,
        needs_ocr: false,
    })
}
