//! Orchestration of case-attached files.
//!
//! When a clinician submits a case, they often bring evidence specific to
//! that patient — analytics PDFs, imaging reports, ECG screenshots, notes.
//! These attachments live with the case, not with the workspace knowledge
//! base, so personal data does not leak into shared retrieval.
//!
//! Pipeline per attachment:
//!
//! 1. Compute sha256 over the bytes.
//! 2. Copy to `workspace_dir/cases/<case_id>/attachments/<sha8>-<slug>.<ext>`
//!    (filename truncated to keep paths sane, mirroring the RAG repository
//!    convention).
//! 3. Run the file through [`extract_from_path`] under `catch_unwind` so a
//!    malformed PDF or unsupported image cannot kill the worker.
//! 4. De-identify the extracted text before persistence so the masked
//!    snippet is what feeds the LLM prompt.
//! 5. Return a [`CaseAttachment`] record (not yet persisted — the caller
//!    decides when to commit, since the attachment is bound to a case id
//!    that may not exist yet).
//!
//! Files are processed concurrently with a bounded semaphore so a 20-file
//! drop does not exhaust file descriptors or block the UI.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use slug::slugify;
use tokio::sync::Semaphore;
use uuid::Uuid;

use conclave_core::{Error, Result};
use conclave_deident::Deidentifier;
use conclave_rag::{extract_from_path, DocType, ExtractedText};

use crate::persistence::CaseAttachment;

/// Maximum concurrent file extractions. Matches the ingest pipeline so the
/// behaviour stays predictable across the app.
const MAX_CONCURRENT: usize = 4;

/// Hard cap on the chars of extracted text we keep per attachment. Long
/// extractions are truncated with an ellipsis so the prompt remains
/// reasonable.
const MAX_EXTRACTED_CHARS: usize = 12_000;

/// Run the extraction pipeline for every path in `paths`, returning one
/// [`CaseAttachment`] per file in input order.
///
/// `case_id` is used for the destination subdirectory and stamped onto
/// every returned record.
///
/// `deidentifier` masks PII from the extracted text *before* it touches
/// disk in the `extracted_text` column, mirroring the case-text invariant.
///
/// Errors from a single file (unsupported extension, IO failure, extractor
/// panic) are surfaced as `Err`; callers may choose to tolerate them and
/// continue with the rest, or fail the whole case.
pub async fn ingest_case_attachments(
    paths: Vec<PathBuf>,
    case_id: &str,
    cases_root: &Path,
    deidentifier: &(dyn Deidentifier + Send + Sync),
) -> Result<Vec<CaseAttachment>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let dst_dir = cases_root.join(case_id).join("attachments");
    tokio::fs::create_dir_all(&dst_dir)
        .await
        .map_err(|e| Error::Rag(format!("create attachments dir: {e}")))?;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut handles = Vec::with_capacity(paths.len());
    for (idx, src) in paths.into_iter().enumerate() {
        let sem = Arc::clone(&semaphore);
        let dst_dir = dst_dir.clone();
        let case_id = case_id.to_owned();
        let handle = tokio::task::spawn(async move {
            let _permit = sem.acquire_owned().await.map_err(|e| {
                Error::Rag(format!("semaphore closed during attachment ingest: {e}"))
            })?;
            ingest_one(src, idx as u32 + 1, &case_id, &dst_dir).await
        });
        handles.push(handle);
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        let raw = h
            .await
            .map_err(|e| Error::Rag(format!("attachment task join: {e}")))??;
        // De-identify the extracted text before it lands anywhere.
        let masked = if raw.extracted_text.is_empty() {
            raw.extracted_text.clone()
        } else {
            deidentifier
                .deidentify(&raw.extracted_text)
                .map(|r| r.masked_text)
                .unwrap_or_else(|_| String::new())
        };
        let trimmed = truncate_chars(&masked, MAX_EXTRACTED_CHARS);
        out.push(CaseAttachment {
            extracted_text: trimmed,
            ..raw
        });
    }
    Ok(out)
}

async fn ingest_one(
    src: PathBuf,
    position: u32,
    case_id: &str,
    dst_dir: &Path,
) -> Result<CaseAttachment> {
    let original_filename = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "attachment".to_owned());

    let bytes = tokio::fs::read(&src)
        .await
        .map_err(|e| Error::Rag(format!("read attachment {}: {e}", src.display())))?;
    let byte_size = bytes.len() as u64;
    let sha256 = hex_digest(&bytes);
    let sha8: String = sha256.chars().take(8).collect();

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment");
    let slug = slugify(stem);
    let slug_trimmed: String = slug.chars().take(40).collect();
    let stored_name = format!("{sha8}-{slug_trimmed}.{ext}");
    let stored_path = dst_dir.join(stored_name);

    if !stored_path.exists() {
        tokio::fs::write(&stored_path, &bytes)
            .await
            .map_err(|e| Error::Rag(format!("write attachment: {e}")))?;
    }

    let (doc_type, extracted_text, needs_ocr) = extract_safely(&stored_path);
    let mime = guess_mime(&ext, doc_type);

    Ok(CaseAttachment {
        id: format!("att-{}", Uuid::new_v4()),
        case_id: case_id.to_owned(),
        position,
        original_filename,
        stored_path: stored_path.to_string_lossy().into_owned(),
        sha256,
        doc_type: doc_type_db_str(doc_type).to_owned(),
        mime,
        extracted_text,
        needs_ocr,
        byte_size,
        created_at: Utc::now(),
    })
}

/// Run the extractor with `catch_unwind` so a panicking decoder degrades
/// to "no text extracted" instead of taking down the worker.
fn extract_safely(path: &Path) -> (DocType, String, bool) {
    let owned = path.to_path_buf();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        extract_from_path(&owned)
    }));
    match result {
        Ok(Ok(ExtractedText {
            content,
            doc_type,
            needs_ocr,
            ..
        })) => (doc_type, content, needs_ocr),
        Ok(Err(err)) => {
            tracing::warn!(path = %path.display(), error = ?err, "attachment extractor returned error");
            (
                DocType::from_path(path).unwrap_or(DocType::Txt),
                String::new(),
                true,
            )
        }
        Err(_) => {
            tracing::warn!(path = %path.display(), "attachment extractor panicked");
            (
                DocType::from_path(path).unwrap_or(DocType::Txt),
                String::new(),
                true,
            )
        }
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

const fn doc_type_db_str(d: DocType) -> &'static str {
    match d {
        DocType::Pdf => "pdf",
        DocType::Docx => "docx",
        DocType::Txt => "txt",
        DocType::Md => "md",
        DocType::Html => "html",
        DocType::Image => "image",
    }
}

fn guess_mime(ext: &str, doc_type: DocType) -> String {
    if matches!(doc_type, DocType::Image) {
        match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "tif" | "tiff" => "image/tiff",
            "heic" | "heif" => "image/heic",
            _ => "application/octet-stream",
        }
        .to_owned()
    } else {
        match ext {
            "pdf" => "application/pdf",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "txt" => "text/plain",
            "md" | "markdown" => "text/markdown",
            "html" | "htm" => "text/html",
            _ => "application/octet-stream",
        }
        .to_owned()
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut end = s.len();
    for (count, (i, _)) in s.char_indices().enumerate() {
        if count == max {
            end = i;
            break;
        }
    }
    if end < s.len() {
        let mut out = s[..end].to_owned();
        out.push('…');
        out
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conclave_deident::PipelineDeidentifier;

    #[tokio::test]
    async fn ingests_text_attachment() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("note.txt");
        tokio::fs::write(&src, "Paciente con HTA. Glucemia 110.")
            .await
            .unwrap();

        let cases_root = tmp.path().join("cases");
        let deid = PipelineDeidentifier::new();
        let attachments =
            ingest_case_attachments(vec![src.clone()], "case-test", &cases_root, &deid)
                .await
                .unwrap();

        assert_eq!(attachments.len(), 1);
        let a = &attachments[0];
        assert_eq!(a.case_id, "case-test");
        assert_eq!(a.position, 1);
        assert_eq!(a.doc_type, "txt");
        assert!(a.extracted_text.contains("HTA"));
        assert!(!a.needs_ocr);
        assert!(!a.sha256.is_empty());
        assert!(a.stored_path.contains("case-test"));
    }

    #[tokio::test]
    async fn image_attachment_marked_needs_ocr() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("ecg.png");
        tokio::fs::write(&src, b"\x89PNG\r\n\x1a\n").await.unwrap();

        let deid = PipelineDeidentifier::new();
        let attachments = ingest_case_attachments(vec![src], "case-img", tmp.path(), &deid)
            .await
            .unwrap();

        assert_eq!(attachments.len(), 1);
        let a = &attachments[0];
        assert_eq!(a.doc_type, "image");
        assert_eq!(a.mime, "image/png");
        assert!(a.needs_ocr);
        assert!(a.extracted_text.is_empty());
    }

    #[tokio::test]
    async fn truncates_long_text_attachment() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("long.txt");
        let body = "x".repeat(MAX_EXTRACTED_CHARS + 5_000);
        tokio::fs::write(&src, body).await.unwrap();

        let deid = PipelineDeidentifier::new();
        let attachments = ingest_case_attachments(vec![src], "case-long", tmp.path(), &deid)
            .await
            .unwrap();
        // truncated text ends with ellipsis; char count is bounded
        assert!(attachments[0].extracted_text.chars().count() <= MAX_EXTRACTED_CHARS + 1);
        assert!(attachments[0].extracted_text.ends_with('…'));
    }

    #[tokio::test]
    async fn assigns_sequential_positions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = tmp.path().join(format!("note-{i}.txt"));
            tokio::fs::write(&p, format!("note {i}")).await.unwrap();
            paths.push(p);
        }
        let deid = PipelineDeidentifier::new();
        let attachments = ingest_case_attachments(paths, "case-seq", tmp.path(), &deid)
            .await
            .unwrap();
        let positions: Vec<u32> = attachments.iter().map(|a| a.position).collect();
        assert_eq!(positions, vec![1, 2, 3]);
    }
}
