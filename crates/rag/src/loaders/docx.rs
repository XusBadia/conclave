//! DOCX loader (OOXML word-processing format).

use std::path::Path;

use conclave_core::{Error, Result};

use super::{Document, DocumentFormat};

/// Load a DOCX file and return its concatenated paragraph text.
pub fn load_docx(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path).map_err(|e| Error::io_at(path, e))?;
    let parsed = docx_rs::read_docx(&bytes)
        .map_err(|e| Error::Rag(format!("docx parse failed for {}: {e}", path.display())))?;
    let text = extract_text(&parsed);
    let title = first_paragraph_as_title(&text).or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.replace(['_', '-'], " "))
            .filter(|s| !s.trim().is_empty())
    });
    Ok(Document::new(path, DocumentFormat::Docx, title, text))
}

fn extract_text(doc: &docx_rs::Docx) -> String {
    use docx_rs::{DocumentChild, ParagraphChild, RunChild};

    let mut paragraphs: Vec<String> = Vec::new();
    for child in &doc.document.children {
        if let DocumentChild::Paragraph(p) = child {
            let mut line = String::new();
            for pchild in &p.children {
                if let ParagraphChild::Run(run) = pchild {
                    for rchild in &run.children {
                        match rchild {
                            RunChild::Text(t) => line.push_str(&t.text),
                            RunChild::Tab(_) => line.push('\t'),
                            RunChild::Break(_) => line.push('\n'),
                            _ => {}
                        }
                    }
                }
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                paragraphs.push(trimmed.to_owned());
            }
        }
    }
    paragraphs.join("\n\n")
}

fn first_paragraph_as_title(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .filter(|l| l.len() <= 120)
        .map(str::to_owned)
}
