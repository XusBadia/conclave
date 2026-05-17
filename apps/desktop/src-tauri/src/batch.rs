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

/// Maximum length of a bare-digit token that we still treat as a
/// patient identifier. Anything longer (e.g. an 8-digit date
/// `20240312`) is rejected so it doesn't pollute the key.
const MAX_ID_NUMERIC_LEN: usize = 4;

/// Propose a grouping for a flat list of file paths the user dropped or
/// picked, using a filename-prefix heuristic. The frontend treats this
/// as an *initial* proposal — the user can always reorganize it in the
/// review screen before running.
///
/// Algorithm (per file):
/// 1. Split the file stem on `-`/`_`/space/`.` into tokens.
/// 2. If any token is a short bare number (≤4 digits, ≥1 digit) it is
///    treated as the **patient identifier numeric**. The key is the
///    tokens from index 0 up to and including that numeric, joined with
///    `-`. This handles "CR-IA-001_recto…" → key `cr-ia-001` and
///    multi-document patients like "CR-IA-011_07_RM_pelvis" (the second
///    `07` is ignored because we stop at the first numeric).
/// 3. If no short numeric token exists, fall back to the **first
///    non-modality, non-bare-numeric alpha token**. This keeps the
///    older `juan-ecg.png` + `juan-labs.pdf` pattern working: both
///    collapse to key `juan`.
/// 4. Files whose key resolves to empty (e.g. `scan.pdf` — only
///    modality tokens) inherit the **first non-empty key** so they
///    join the first identified patient rather than form a phantom
///    group.
///
/// Output groups preserve drop-order for stable rendering.
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
        /// Display label derived from the same tokens as the key, with
        /// the user's casing preserved (e.g. "CR-IA-001" rather than
        /// "cr-ia-001").
        display_label: String,
    }
    let mut entries: Vec<Entry> = paths
        .into_iter()
        .map(|p| {
            let original_stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            let (key, display_label) = extract_patient_key_and_label(&original_stem);
            Entry {
                path: p,
                original_stem,
                key,
                display_label,
            }
        })
        .collect();

    // Empty keys all the way down (every file is a generic modality
    // name like `ecg.png`, `scan.pdf`) → fold into one case.
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

    // Lone generic files (empty key) inherit the first identified key
    // so they join that patient rather than form their own group.
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
        // Prefer a non-empty display_label from one of the contributing
        // files; longer wins (preserves the fully-cased patient id).
        let label = idxs
            .iter()
            .map(|&i| entries[i].display_label.clone())
            .filter(|s| !s.is_empty())
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

/// Extract a `(key, display_label)` pair from a filename stem.
///
/// `key` is lowercase + `-`-joined for HashMap grouping. `display_label`
/// is the same tokens with the user's original casing — used as the
/// initial patient label in the review dialog.
fn extract_patient_key_and_label(stem: &str) -> (String, String) {
    let raw_tokens: Vec<&str> = stem
        .split(['-', '_', ' ', '.'])
        .filter(|t| !t.is_empty())
        .collect();
    if raw_tokens.is_empty() {
        return (String::new(), String::new());
    }

    // Patient identifier with embedded numeric (CR-IA-001, MRN-204711, etc).
    // We take everything from token 0 up to and including the first numeric
    // token that's short enough to be a case index (≤ MAX_ID_NUMERIC_LEN
    // digits). Longer pure-digit tokens (8-digit dates, etc.) are skipped.
    let first_id_numeric = raw_tokens.iter().position(|t| {
        let len = t.len();
        (1..=MAX_ID_NUMERIC_LEN).contains(&len) && t.chars().all(|c| c.is_ascii_digit())
    });
    if let Some(idx) = first_id_numeric {
        let id_tokens = &raw_tokens[..=idx];
        let key = id_tokens
            .iter()
            .map(|t| t.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("-");
        let label = id_tokens.join("-");
        return (key, label);
    }

    // No numeric ID token: fall back to the first non-modality,
    // non-bare-digit alpha token (the `juan-ecg` pattern).
    for t in &raw_tokens {
        if t.chars().all(|c| c.is_ascii_digit()) {
            continue; // longer-than-MAX_ID date-like number — skip
        }
        let lc = t.to_ascii_lowercase();
        if MODALITY_TOKENS.contains(&lc.as_str()) {
            continue;
        }
        return (lc, (*t).to_owned());
    }

    (String::new(), String::new())
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

    #[test]
    fn propose_groups_by_alphanumeric_id_prefix() {
        // Reproduces the real-world MDT colorectal naming: most files
        // are single-document (CR-IA-001 .. CR-IA-010) and CR-IA-011 has
        // 8 documents with a numeric document index AFTER the patient
        // id. The first numeric token (≤4 digits) is the patient id; the
        // second numeric (e.g. `07` in `CR-IA-011_07_RM_pelvis`) is the
        // document index and must NOT split the group.
        let cases = propose_grouping_from_files(
            vec![
                p("CR-IA-001_recto_bajo_alto_riesgo.pdf"),
                p("CR-IA-002_recto_cCR_post_TNT.pdf"),
                p("CR-IA-003_polipo_maligno_T1_sigma.pdf"),
                p("CR-IA-011_00_peticion_comite_colorrectal.pdf"),
                p("CR-IA-011_01_consulta_inicial_digestivo.pdf"),
                p("CR-IA-011_07_RM_pelvis_protocolo_recto.pdf"),
            ],
            "Q",
        );
        assert_eq!(cases.len(), 4, "001/002/003 each + a single 011 group");
        let cr011 = cases
            .iter()
            .find(|c| c.patient_label.contains("011"))
            .expect("CR-IA-011 group missing");
        assert_eq!(
            cr011.attached_file_paths.len(),
            3,
            "all three CR-IA-011 files should share a group",
        );
        // Label keeps the case sensitivity of the original filename.
        assert_eq!(cr011.patient_label, "CR-IA-011");
        let cr001 = cases
            .iter()
            .find(|c| c.patient_label.contains("001"))
            .expect("CR-IA-001 group missing");
        assert_eq!(cr001.attached_file_paths.len(), 1);
        assert_eq!(cr001.patient_label, "CR-IA-001");
    }

    #[test]
    fn propose_ignores_long_numeric_tokens_as_id() {
        // 8-digit numeric tokens look like dates, not patient ids.
        // The fallback path should still find a name-style key.
        let cases = propose_grouping_from_files(
            vec![p("juan-20240312-ecg.pdf"), p("juan-20240315-labs.pdf")],
            "Q",
        );
        // Both files share the alpha prefix `juan`; the 8-digit date is
        // skipped as a candidate patient identifier.
        assert_eq!(cases.len(), 1, "both should collapse under `juan`");
        assert_eq!(cases[0].attached_file_paths.len(), 2);
    }
}
