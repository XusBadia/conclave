//! Markdown and plain-text loaders.

use std::path::Path;

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use conclave_core::{Error, Result};

use super::{Document, DocumentFormat};

/// Load a plain `.txt` file verbatim.
pub fn load_plain_text(path: &Path) -> Result<Document> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    let title = file_stem_title(path);
    Ok(Document::new(
        path,
        DocumentFormat::PlainText,
        title,
        raw.trim().to_owned(),
    ))
}

/// Load a Markdown file and produce a normalised plain-text representation.
///
/// Code fences, link URLs and HTML pass-through are preserved as text;
/// headings are kept (without the `#` markers). This keeps clinical
/// boilerplate (drug names inside code spans, dosages inside tables) intact
/// while still letting the FTS5 tokenizer see them as ordinary words.
pub fn load_markdown(path: &Path) -> Result<Document> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    let (title, body) = render_markdown(&raw);
    Ok(Document::new(
        path,
        DocumentFormat::Markdown,
        title.or_else(|| file_stem_title(path)),
        body,
    ))
}

fn file_stem_title(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace(['_', '-'], " "))
        .filter(|s| !s.trim().is_empty())
}

fn render_markdown(raw: &str) -> (Option<String>, String) {
    let parser = Parser::new(raw);
    let mut out = String::with_capacity(raw.len());
    let mut title: Option<String> = None;
    let mut in_h1 = false;
    let mut h1_buf = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if level == HeadingLevel::H1 && title.is_none() {
                    in_h1 = true;
                    h1_buf.clear();
                }
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if in_h1 {
                    title = Some(h1_buf.trim().to_owned());
                    in_h1 = false;
                }
                out.push('\n');
            }
            Event::Start(Tag::Paragraph | Tag::Item | Tag::CodeBlock(_))
            | Event::End(TagEnd::CodeBlock) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Item) | Event::SoftBreak | Event::HardBreak => {
                out.push('\n');
            }
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if in_h1 {
                    h1_buf.push_str(&t);
                }
                out.push_str(&t);
            }
            Event::Rule => out.push_str("\n---\n"),
            _ => {}
        }
    }

    let trimmed = out.trim().to_owned();
    (title.filter(|t| !t.is_empty()), trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("note.txt");
        std::fs::write(&path, "  hola\nmundo  \n").unwrap();
        let doc = load_plain_text(&path).unwrap();
        assert_eq!(doc.text, "hola\nmundo");
        assert_eq!(doc.title.as_deref(), Some("note"));
        assert_eq!(doc.format, DocumentFormat::PlainText);
    }

    #[test]
    fn markdown_extracts_title_and_text() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("guideline.md");
        std::fs::write(
            &path,
            "# Manejo del IAMCEST\n\nReperfusión en <120 minutos.\n\n## Antiagregación\n\nAAS 300 mg.\n",
        )
        .unwrap();
        let doc = load_markdown(&path).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Manejo del IAMCEST"));
        assert!(doc.text.contains("Reperfusión"));
        assert!(doc.text.contains("AAS 300 mg"));
    }

    #[test]
    fn markdown_falls_back_to_filename_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("infarto_agudo.md");
        std::fs::write(&path, "No heading here, just prose.\n").unwrap();
        let doc = load_markdown(&path).unwrap();
        assert_eq!(doc.title.as_deref(), Some("infarto agudo"));
    }
}
