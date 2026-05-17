//! Batch case ingestion: read a folder where each subdirectory represents
//! one patient, returning a normalized list of `BatchCaseInput` records
//! the frontend can preview and edit before triggering the actual run.
//!
//! Folder layout we expect:
//!
//! ```text
//! root/
//! ├── patient-1/
//! │   ├── case.txt          # optional clinical note (also .md)
//! │   ├── labs.pdf
//! │   └── ecg.png
//! └── patient-2/
//!     └── case.md
//! ```
//!
//! If the user drags a folder containing loose files instead of patient
//! subfolders, we degrade gracefully and treat each file as its own
//! one-attachment case so the UX still works.
//!
//! This module is intentionally simple and **not** the place where files
//! get extracted or de-identified — that happens later, per-case, in the
//! `ingest_case_attachments` orchestrator. We only do filesystem walking
//! here so the parse can run on the foreground thread without surprise IO.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One patient parsed out of the batch folder. Mirrors the IPC type so
/// commands.rs can pass it straight through to the frontend preview.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchCaseInput {
    pub patient_label: String,
    /// Contents of `case.txt`/`case.md` if present, else empty.
    pub text: String,
    /// Default question to apply — frontend may override per-row.
    pub question: String,
    /// Absolute paths of every attachment file (excluding case.txt/.md).
    pub attached_file_paths: Vec<String>,
}

/// Recognised "narrative" filenames inside a patient folder, in priority
/// order. The first match wins; everything else stays as an attachment.
const NARRATIVE_NAMES: &[&str] = &[
    "case.md",
    "case.txt",
    "caso.md",
    "caso.txt",
    "patient.md",
    "patient.txt",
];

/// Extensions we recognise as attachments. Anything else is silently
/// dropped (instead of failing) so the user does not have to clean up
/// `.DS_Store` / system files by hand.
const ATTACHMENT_EXTS: &[&str] = &[
    "pdf", "docx", "txt", "md", "markdown", "html", "htm", "png", "jpg", "jpeg", "webp", "tif",
    "tiff", "heic", "heif",
];

/// Parse a folder into a list of `BatchCaseInput` records.
///
/// - If `root` has at least one subdirectory, each subdirectory becomes
///   one patient. Loose files at the top level are ignored.
/// - Otherwise (folder contains only files), each individual file becomes
///   a one-attachment case named after the file stem.
///
/// `default_question` is stamped on every record so the frontend has a
/// sensible starting point without round-tripping translations from Rust.
pub fn parse_batch_folder(
    root: &Path,
    default_question: &str,
) -> Result<Vec<BatchCaseInput>, String> {
    let meta = std::fs::metadata(root).map_err(|e| format!("stat {}: {e}", root.display()))?;
    if !meta.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let entries = std::fs::read_dir(root)
        .map_err(|e| format!("read_dir {}: {e}", root.display()))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();

    let has_subdirs = entries.iter().any(|e| e.path().is_dir());

    let mut cases: Vec<BatchCaseInput> = Vec::new();
    if has_subdirs {
        let mut subdirs: Vec<PathBuf> = entries
            .iter()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subdirs.sort();
        for subdir in subdirs {
            let label = subdir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "patient".to_owned());
            cases.push(parse_one_subdir(&label, &subdir, default_question)?);
        }
    } else {
        // Loose-files mode: one case per file.
        let mut files: Vec<PathBuf> = entries
            .iter()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_attachment(p))
            .collect();
        files.sort();
        for file in files {
            let label = file
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "case".to_owned());
            cases.push(BatchCaseInput {
                patient_label: label,
                text: String::new(),
                question: default_question.to_owned(),
                attached_file_paths: vec![file.to_string_lossy().into_owned()],
            });
        }
    }

    Ok(cases)
}

fn parse_one_subdir(
    label: &str,
    dir: &Path,
    default_question: &str,
) -> Result<BatchCaseInput, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();

    // Pick a narrative file if present.
    let narrative_path = entries.iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| {
                NARRATIVE_NAMES
                    .iter()
                    .any(|cand| cand.eq_ignore_ascii_case(n))
            })
            .unwrap_or(false)
    });

    let text = if let Some(p) = narrative_path {
        std::fs::read_to_string(p).unwrap_or_default()
    } else {
        String::new()
    };

    let mut attachments: Vec<String> = Vec::new();
    for entry in &entries {
        if Some(entry) == narrative_path {
            continue;
        }
        if !is_attachment(entry) {
            continue;
        }
        attachments.push(entry.to_string_lossy().into_owned());
    }
    attachments.sort();

    Ok(BatchCaseInput {
        patient_label: label.to_owned(),
        text,
        question: default_question.to_owned(),
        attached_file_paths: attachments,
    })
}

fn is_attachment(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            ATTACHMENT_EXTS.iter().any(|x| *x == lower)
        })
        .unwrap_or(false)
}

/// Tokens we strip from filename stems before grouping because they are
/// modalities or document types, not patient identifiers. The heuristic
/// works in lowercase; entries here are also lowercase.
const MODALITY_TOKENS: &[&str] = &[
    "analitica",
    "analytica",
    "analytics",
    "labs",
    "lab",
    "bloodwork",
    "blood",
    "ecg",
    "ekg",
    "xray",
    "x-ray",
    "xr",
    "rx",
    "tac",
    "ct",
    "rm",
    "rmn",
    "mri",
    "informe",
    "report",
    "consulta",
    "consult",
    "notes",
    "notas",
    "note",
    "pdf",
    "img",
    "image",
    "scan",
    "results",
    "result",
    "summary",
    "summarised",
    "discharge",
    "alta",
    "ingreso",
    "admission",
    "visit",
    "visita",
    "case",
    "caso",
    "patient",
    "paciente",
    "v1",
    "v2",
    "v3",
    "final",
    "draft",
    "copy",
    "copia",
];

/// Propose a grouping for a flat list of file paths the user dropped or
/// picked, using a filename-prefix heuristic. The frontend treats this as
/// an *initial* proposal — the user can always reorganize it in the
/// review screen before running.
///
/// Algorithm:
/// 1. Compute a "normalized stem" per file: lowercased file stem with
///    separators unified to `-`, modality / document-type tokens stripped,
///    bare digit runs stripped, dates collapsed. The result is intended
///    to be the patient identifier hiding inside the filename.
/// 2. If every file collapses to the same (or empty) stem → one group.
/// 3. Otherwise → one group per distinct stem, label = first original
///    file-stem encountered for that key (preserves the user's casing).
///
/// The heuristic is intentionally aggressive (a `juan-ecg.png` + `juan-labs.pdf`
/// pair will collapse to one patient even though their stems are different
/// raw) — the review screen makes the auto-decision plainly editable.
pub fn propose_grouping_from_files(
    paths: Vec<PathBuf>,
    default_question: &str,
) -> Vec<BatchCaseInput> {
    if paths.is_empty() {
        return Vec::new();
    }

    // Per-file stem keys, preserving original order for stable output.
    #[derive(Debug)]
    struct Entry {
        path: PathBuf,
        original_stem: String,
        key: String,
    }
    let mut entries: Vec<Entry> = paths
        .into_iter()
        .map(|p| {
            let original_stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            let key = normalize_stem(&original_stem);
            Entry {
                path: p,
                original_stem,
                key,
            }
        })
        .collect();

    // Empty keys are merged together — they all look like generic
    // "PDF", "scan", "report" style filenames with no identifier.
    let all_empty = entries.iter().all(|e| e.key.is_empty());
    if all_empty {
        let label = entries
            .first()
            .map(|e| {
                if e.original_stem.is_empty() {
                    "Paciente 1".to_owned()
                } else {
                    e.original_stem.clone()
                }
            })
            .unwrap_or_else(|| "Paciente 1".to_owned());
        return vec![BatchCaseInput {
            patient_label: label,
            text: String::new(),
            question: default_question.to_owned(),
            attached_file_paths: entries
                .into_iter()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect(),
        }];
    }

    // Files whose normalized stem is empty inherit the key of the
    // **first non-empty key** seen so they don't form a phantom group by
    // themselves. Practically this means a lone `scan.pdf` dropped
    // alongside `juan-ecg.png` joins "juan", which is usually what the
    // clinician meant.
    let first_real_key = entries
        .iter()
        .find(|e| !e.key.is_empty())
        .map(|e| e.key.clone())
        .unwrap_or_default();
    for e in &mut entries {
        if e.key.is_empty() {
            e.key.clone_from(&first_real_key);
        }
    }

    // Group preserving first-seen order so the resulting cases follow the
    // user's drag order.
    let mut order: Vec<String> = Vec::new();
    let mut by_key: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if !by_key.contains_key(&e.key) {
            order.push(e.key.clone());
        }
        by_key.entry(e.key.clone()).or_default().push(i);
    }

    let mut out: Vec<BatchCaseInput> = Vec::with_capacity(order.len());
    for key in order {
        let idxs = by_key.remove(&key).unwrap_or_default();
        // Pick the most representative original stem for the label: the
        // longest one wins (it's the least stripped).
        let label = idxs
            .iter()
            .map(|&i| entries[i].original_stem.clone())
            .max_by_key(|s| s.len())
            .unwrap_or_else(|| key.clone());
        let attached_file_paths: Vec<String> = idxs
            .into_iter()
            .map(|i| entries[i].path.to_string_lossy().into_owned())
            .collect();
        out.push(BatchCaseInput {
            patient_label: if label.is_empty() {
                format!("Paciente {}", out.len() + 1)
            } else {
                label
            },
            text: String::new(),
            question: default_question.to_owned(),
            attached_file_paths,
        });
    }
    out
}

/// Lowercase, unify separators, strip modality tokens + bare digit runs,
/// and trim leading/trailing separators so two files with the same
/// patient identifier collapse to the same key.
fn normalize_stem(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    // Unify common separators to `-`.
    let unified: String = lower
        .chars()
        .map(|c| match c {
            ' ' | '_' | '.' => '-',
            _ => c,
        })
        .collect();
    // Token-by-token strip.
    let kept: Vec<&str> = unified
        .split('-')
        .filter(|tok| !tok.is_empty())
        .filter(|tok| !MODALITY_TOKENS.contains(tok))
        .filter(|tok| !is_bare_number(tok))
        .collect();
    kept.join("-")
}

fn is_bare_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn parses_subfolders_as_patients() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("alice").join("case.txt"), "Alice notes");
        write(&tmp.path().join("alice").join("labs.pdf"), "%PDF-");
        write(&tmp.path().join("bob").join("case.md"), "# Bob");
        // Random non-attachment file should be ignored silently.
        write(&tmp.path().join("bob").join(".DS_Store"), "");

        let cases = parse_batch_folder(tmp.path(), "Manejo?").unwrap();
        assert_eq!(cases.len(), 2);

        let alice = cases.iter().find(|c| c.patient_label == "alice").unwrap();
        assert_eq!(alice.text, "Alice notes");
        assert_eq!(alice.attached_file_paths.len(), 1);
        assert!(alice.attached_file_paths[0].ends_with("labs.pdf"));
        assert_eq!(alice.question, "Manejo?");

        let bob = cases.iter().find(|c| c.patient_label == "bob").unwrap();
        assert!(bob.text.starts_with("# Bob"));
        assert_eq!(bob.attached_file_paths.len(), 0);
    }

    #[test]
    fn falls_back_to_loose_files_mode() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("patient-1.pdf"), "%PDF-");
        write(&tmp.path().join("patient-2.png"), "PNG-fake");
        // .DS_Store and other junk filtered.
        write(&tmp.path().join(".DS_Store"), "");

        let cases = parse_batch_folder(tmp.path(), "Q").unwrap();
        assert_eq!(cases.len(), 2);
        for c in &cases {
            assert!(c.text.is_empty());
            assert_eq!(c.attached_file_paths.len(), 1);
        }
    }

    #[test]
    fn errors_on_missing_directory() {
        let err =
            parse_batch_folder(Path::new("/tmp/conclave-does-not-exist-xyzzy"), "Q").unwrap_err();
        assert!(err.contains("stat") || err.contains("not a directory"));
    }

    #[test]
    fn empty_directory_returns_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let cases = parse_batch_folder(tmp.path(), "Q").unwrap();
        assert_eq!(cases.len(), 0);
    }

    fn p(name: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/{name}"))
    }

    #[test]
    fn propose_groups_juan_and_maria() {
        let cases = propose_grouping_from_files(
            vec![p("juan-ecg.png"), p("juan-labs.pdf"), p("maria-ecg.png")],
            "Q",
        );
        assert_eq!(cases.len(), 2);
        let juan = cases
            .iter()
            .find(|c| c.patient_label.to_lowercase().contains("juan"))
            .expect("expected a juan group");
        assert_eq!(juan.attached_file_paths.len(), 2);
        let maria = cases
            .iter()
            .find(|c| c.patient_label.to_lowercase().contains("maria"))
            .expect("expected a maria group");
        assert_eq!(maria.attached_file_paths.len(), 1);
    }

    #[test]
    fn propose_collapses_modality_tokens() {
        let cases = propose_grouping_from_files(
            vec![
                p("juan-ecg.png"),
                p("juan-analitica.pdf"),
                p("juan-informe.txt"),
            ],
            "Q",
        );
        // All three files share the patient identifier "juan" after
        // modality tokens are stripped.
        assert_eq!(cases.len(), 1);
        assert!(cases[0].patient_label.to_lowercase().contains("juan"));
        assert_eq!(cases[0].attached_file_paths.len(), 3);
    }

    #[test]
    fn propose_single_file_returns_one_case() {
        let cases = propose_grouping_from_files(vec![p("solo.pdf")], "Q");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].attached_file_paths.len(), 1);
        assert_eq!(cases[0].question, "Q");
    }

    #[test]
    fn propose_unrelated_files_returns_separate_cases() {
        let cases =
            propose_grouping_from_files(vec![p("juan.pdf"), p("maria.pdf"), p("luis.pdf")], "Q");
        assert_eq!(cases.len(), 3);
        let labels: Vec<String> = cases
            .iter()
            .map(|c| c.patient_label.to_lowercase())
            .collect();
        assert!(labels.iter().any(|l| l.contains("juan")));
        assert!(labels.iter().any(|l| l.contains("maria")));
        assert!(labels.iter().any(|l| l.contains("luis")));
    }

    #[test]
    fn propose_all_generic_filenames_collapses_to_one_case() {
        // Every filename is just a modality / generic word — there is no
        // patient identifier the heuristic can lock onto. The safe thing
        // is to keep them as one case.
        let cases = propose_grouping_from_files(
            vec![p("ecg.png"), p("analitica.pdf"), p("informe.txt")],
            "Q",
        );
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].attached_file_paths.len(), 3);
    }

    #[test]
    fn propose_empty_input_returns_empty_list() {
        let cases = propose_grouping_from_files(Vec::new(), "Q");
        assert_eq!(cases.len(), 0);
    }

    #[test]
    fn propose_lone_generic_file_merges_into_identified_group() {
        // `scan.pdf` has no patient identifier; it should join the first
        // identified group (`juan`) rather than spawn its own phantom case.
        let cases = propose_grouping_from_files(
            vec![p("juan-ecg.png"), p("scan.pdf"), p("juan-labs.pdf")],
            "Q",
        );
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].attached_file_paths.len(), 3);
    }

    #[test]
    fn propose_underscores_and_spaces_treated_as_dashes() {
        let cases = propose_grouping_from_files(
            vec![p("juan_perez ecg.png"), p("juan-perez-labs.pdf")],
            "Q",
        );
        assert_eq!(
            cases.len(),
            1,
            "underscores/spaces should unify with dashes"
        );
        assert_eq!(cases[0].attached_file_paths.len(), 2);
    }
}
