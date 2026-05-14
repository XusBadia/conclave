//! HTML loader: extracts the visible text and the `<title>` element.

use std::path::Path;

use scraper::{Html, Selector};

use conclave_core::{Error, Result};

use super::{Document, DocumentFormat};

/// Load an HTML file and return its visible text plus the `<title>` element.
pub fn load_html(path: &Path) -> Result<Document> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::io_at(path, e))?;
    let (title, body) = render_html(&raw);
    Ok(Document::new(path, DocumentFormat::Html, title, body))
}

fn render_html(raw: &str) -> (Option<String>, String) {
    let doc = Html::parse_document(raw);
    let title = title_selector()
        .and_then(|sel| doc.select(&sel).next())
        .map(|n| n.text().collect::<String>().trim().to_owned())
        .filter(|s| !s.is_empty());

    // Strip script/style content (`html2text` already does this for us, but
    // we re-render through it to get a sensible block layout).
    let body = html2text::from_read(raw.as_bytes(), 100).trim().to_owned();
    (title, body)
}

fn title_selector() -> Option<Selector> {
    Selector::parse("title").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_and_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("article.html");
        std::fs::write(
            &path,
            r"<!doctype html>
<html><head><title>Manejo del IAMCEST</title></head>
<body>
  <h1>Resumen</h1>
  <p>Reperfusión en menos de <strong>120 minutos</strong>.</p>
  <script>alert('x')</script>
</body></html>",
        )
        .unwrap();
        let doc = load_html(&path).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Manejo del IAMCEST"));
        assert!(doc.text.contains("Reperfusión"));
        assert!(doc.text.contains("120 minutos"));
        assert!(!doc.text.contains("alert"));
    }
}
