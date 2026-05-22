//! SQLite-backed persistence for cases, verdicts, retrieval traces and
//! feedback. Lives in the same `metadata.sqlite` as document metadata so
//! a single workspace database holds everything.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

use crate::privacy::{sha256_hex, AuditPayloadMode, DataBoundaryMode, RawTextRetention};

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS cases (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    question TEXT NOT NULL,
    original_text TEXT NOT NULL,
    masked_text TEXT NOT NULL,
    deident_pipeline_id TEXT NOT NULL,
    status TEXT NOT NULL,
    case_date TEXT NOT NULL DEFAULT '',
    patient_label TEXT NOT NULL DEFAULT '',
    latest_error TEXT,
    raw_text_sha256 TEXT NOT NULL DEFAULT '',
    raw_text_retention TEXT NOT NULL DEFAULT 'discarded'
);

CREATE INDEX IF NOT EXISTS cases_created_at_idx ON cases(created_at DESC);
-- The cases_case_date_idx index lives in `migrate_cases_case_date` because
-- legacy DBs need ALTER TABLE before the index column exists.

CREATE TABLE IF NOT EXISTS verdicts (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    prompt_version TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    latency_ms INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    output_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS verdicts_case_id_idx ON verdicts(case_id);

CREATE TABLE IF NOT EXISTS retrieval_traces (
    verdict_id TEXT PRIMARY KEY REFERENCES verdicts(id) ON DELETE CASCADE,
    evidence_refs_json TEXT NOT NULL,
    past_cases_refs_json TEXT NOT NULL,
    online_evidence_refs_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS feedback (
    case_id TEXT PRIMARY KEY REFERENCES cases(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    reason TEXT,
    modified_verdict_json TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS case_memory (
    case_id TEXT PRIMARY KEY REFERENCES cases(id) ON DELETE CASCADE,
    embedding BLOB NOT NULL,
    case_summary TEXT NOT NULL,
    verdict_summary TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS case_attachments (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    original_filename TEXT NOT NULL,
    stored_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    doc_type TEXT NOT NULL,
    mime TEXT NOT NULL DEFAULT '',
    extracted_text TEXT NOT NULL,
    needs_ocr INTEGER NOT NULL DEFAULT 0,
    byte_size INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS case_attachments_case_idx
    ON case_attachments(case_id, position);

CREATE TABLE IF NOT EXISTS deliberation_traces (
    id TEXT PRIMARY KEY,
    verdict_id TEXT NOT NULL REFERENCES verdicts(id) ON DELETE CASCADE,
    briefing_output TEXT,
    drafting_output TEXT,
    redteam_output TEXT,
    total_input_tokens INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    vision_used INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS deliberation_traces_verdict_idx
    ON deliberation_traces(verdict_id);

CREATE TABLE IF NOT EXISTS review_metadata (
    case_id TEXT PRIMARY KEY REFERENCES cases(id) ON DELETE CASCADE,
    verdict_id TEXT NOT NULL REFERENCES verdicts(id) ON DELETE CASCADE,
    decision TEXT NOT NULL,
    reviewer_name TEXT,
    reviewer_role TEXT,
    note TEXT,
    final_verdict_json TEXT,
    diff_summary TEXT,
    reviewed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_runs (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    verdict_id TEXT,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    data_boundary_mode TEXT NOT NULL,
    payload_mode TEXT NOT NULL,
    active_skill_id TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    prompt_sha256 TEXT NOT NULL DEFAULT '',
    output_sha256 TEXT NOT NULL DEFAULT '',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    past_cases_refs_json TEXT NOT NULL DEFAULT '[]',
    online_evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    attachment_refs_json TEXT NOT NULL DEFAULT '[]',
    raw_text_retention TEXT NOT NULL DEFAULT 'discarded',
    status TEXT NOT NULL,
    error TEXT
);

CREATE INDEX IF NOT EXISTS audit_runs_case_idx
    ON audit_runs(case_id, started_at DESC);
";

/// Outcome flag for a case lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    /// Case created with attachments but no verdict yet. The clinician
    /// can later open it from the list to add clinical context and run
    /// the committee — at which point it becomes `Completed` or `Failed`.
    Draft,
    /// Verdict produced and persisted.
    ReviewReady,
    /// Clinician explicitly reviewed and finalized the verdict.
    Finalized,
    /// Legacy row that was created under the old `completed` semantics.
    FinalizedLegacy,
    /// LLM call or validation failed.
    Failed,
}

impl CaseStatus {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::ReviewReady => "review_ready",
            Self::Finalized => "finalized",
            Self::FinalizedLegacy => "finalized_legacy",
            Self::Failed => "failed",
        }
    }
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "completed" | "finalized_legacy" => Some(Self::FinalizedLegacy),
            "review_ready" => Some(Self::ReviewReady),
            "finalized" => Some(Self::Finalized),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Stored case row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    /// User-facing date of the clinical consultation. Editable from the UI;
    /// defaults to `created_at` on insert. Used as the primary sort key
    /// in the cases list so backdated rows surface in the right slot.
    pub case_date: DateTime<Utc>,
    pub workspace_id: String,
    pub question: String,
    pub original_text: String,
    pub masked_text: String,
    pub deident_pipeline_id: String,
    pub status: CaseStatus,
    /// Human-friendly identifier for the case (e.g. "Juan Pérez",
    /// "CR-IA-011"). Used as the row title in the list so multi-case
    /// batches do not all look identical. Free-text — empty falls back to
    /// the question or the case id.
    #[serde(default)]
    pub patient_label: String,
    /// Error message captured when `status == Failed`. Surfaced in the
    /// detail view so the clinician sees *why* the committee aborted
    /// instead of an opaque "failed" badge.
    #[serde(default)]
    pub latest_error: Option<String>,
    /// Stable fingerprint of the original text, even after raw narrative is
    /// purged.
    #[serde(default)]
    pub raw_text_sha256: String,
    /// Local retention posture for `original_text`.
    #[serde(default = "default_raw_text_retention")]
    pub raw_text_retention: RawTextRetention,
}

fn default_raw_text_retention() -> RawTextRetention {
    RawTextRetention::Discarded
}

/// Stored verdict row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub id: String,
    pub case_id: String,
    pub prompt_version: String,
    pub provider_id: String,
    pub model: String,
    pub latency_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub output_json: String,
    pub created_at: DateTime<Utc>,
}

/// Retrieval trace: which evidence ids fed the prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalTrace {
    pub verdict_id: String,
    pub evidence_refs: Vec<String>,
    pub past_cases_refs: Vec<String>,
    pub online_evidence_refs: Vec<String>,
    /// Case attachment refs (`A1..AN`) supplied to the prompt.
    #[serde(default)]
    pub attachment_refs: Vec<String>,
}

/// Full record of a deliberative run — the three intermediate outputs the
/// committee produced before the final verdict was emitted. Persisted only
/// when the user runs in deliberative mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliberationTrace {
    pub id: String,
    pub verdict_id: String,
    /// Phase 1 — markdown briefing of evidence + gaps.
    pub briefing_output: Option<String>,
    /// Phase 2 — JSON draft verdict produced from the briefing.
    pub drafting_output: Option<String>,
    /// Phase 3 — markdown adversarial critique of the draft.
    pub redteam_output: Option<String>,
    /// Sum of `input_tokens` reported by every phase call.
    pub total_input_tokens: u32,
    /// Sum of `output_tokens` reported by every phase call.
    pub total_output_tokens: u32,
    /// Wall-clock duration of the whole deliberation in ms.
    pub duration_ms: u64,
    /// `true` if at least one phase forwarded images to a vision-capable
    /// provider. The UI surfaces this so the clinician knows the images
    /// were actually interpreted (not just listed as attachments).
    pub vision_used: bool,
    pub created_at: DateTime<Utc>,
}

/// One file attached to a specific case (analytics, ECG image, lab PDF…).
///
/// Attachments live with their case; they are never copied into the
/// workspace knowledge base, so personal data does not leak into shared
/// evidence retrieval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseAttachment {
    pub id: String,
    pub case_id: String,
    /// 1-based slot used to render the `[A{position}]` ref in the prompt.
    pub position: u32,
    pub original_filename: String,
    pub stored_path: String,
    pub sha256: String,
    pub doc_type: String,
    /// MIME type, when known (e.g. `image/png`). Useful for vision routing.
    #[serde(default)]
    pub mime: String,
    /// Text recovered from the file. Empty when the file is an image
    /// without OCR; the UI must label this honestly.
    pub extracted_text: String,
    /// `true` when the file is a raster/image whose text could not be
    /// recovered. Vision-capable providers can still interpret the bytes
    /// directly.
    pub needs_ocr: bool,
    pub byte_size: u64,
    pub created_at: DateTime<Utc>,
}

/// User feedback on a case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub case_id: String,
    pub kind: FeedbackKind,
    pub reason: Option<String>,
    pub modified_verdict_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Feedback flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    Accept,
    Modify,
    Reject,
}

/// Review/finalization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewDecision {
    Accept,
    Modify,
    Reject,
}

impl ReviewDecision {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Modify => "modify",
            Self::Reject => "reject",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "accept" => Some(Self::Accept),
            "modify" => Some(Self::Modify),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewMetadataRecord {
    pub case_id: String,
    pub verdict_id: String,
    pub decision: ReviewDecision,
    pub reviewer_name: Option<String>,
    pub reviewer_role: Option<String>,
    pub note: Option<String>,
    pub final_verdict_json: Option<String>,
    pub diff_summary: Option<String>,
    pub reviewed_at: DateTime<Utc>,
}

/// One fingerprint-first audit run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRunRecord {
    pub id: String,
    pub case_id: String,
    pub verdict_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub data_boundary_mode: DataBoundaryMode,
    pub payload_mode: AuditPayloadMode,
    pub active_skill_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latency_ms: u64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub prompt_sha256: String,
    pub output_sha256: String,
    pub evidence_refs: Vec<String>,
    pub past_cases_refs: Vec<String>,
    pub online_evidence_refs: Vec<String>,
    pub attachment_refs: Vec<String>,
    pub raw_text_retention: RawTextRetention,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditStatus {
    pub run_count: u64,
    pub payload_mode: AuditPayloadMode,
    pub retained_raw_cases: u64,
    pub legacy_retained_cases: u64,
}

impl FeedbackKind {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Modify => "modify",
            Self::Reject => "reject",
        }
    }
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "accept" => Some(Self::Accept),
            "modify" => Some(Self::Modify),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }
}

/// On-disk store for cases and verdicts.
#[derive(Debug)]
pub struct CaseStore {
    conn: Connection,
    path: PathBuf,
}

impl CaseStore {
    /// Open or create the case database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path).map_err(map_sql)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_sql)?;
        conn.execute_batch(SCHEMA_SQL).map_err(map_sql)?;
        Self::migrate_retrieval_traces_attachments(&conn)?;
        Self::migrate_cases_case_date(&conn)?;
        Self::migrate_cases_patient_label_and_error(&conn)?;
        Self::migrate_cases_privacy(&conn)?;
        Ok(Self { conn, path })
    }

    /// Idempotent migration: pre-existing DBs do not have the
    /// `attachment_refs_json` column on `retrieval_traces`. Add it on first
    /// open and default existing rows to `[]`.
    fn migrate_retrieval_traces_attachments(conn: &Connection) -> Result<()> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(retrieval_traces)")
            .map_err(map_sql)?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(map_sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql)?;
        if !cols.iter().any(|c| c == "attachment_refs_json") {
            conn.execute(
                "ALTER TABLE retrieval_traces \
                 ADD COLUMN attachment_refs_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .map_err(map_sql)?;
        }
        Ok(())
    }

    /// Idempotent migration: add the `case_date` column to `cases` if it
    /// does not exist yet, then backfill any row whose `case_date` is empty
    /// — covering both the very first open of an old DB AND any future
    /// rows accidentally inserted by an older binary version that still
    /// uses the 8-column INSERT (SQLite would accept that thanks to the
    /// `DEFAULT ''`, but those rows would otherwise be unsortable).
    fn migrate_cases_case_date(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(cases)").map_err(map_sql)?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(map_sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql)?;
        if !cols.iter().any(|c| c == "case_date") {
            conn.execute(
                "ALTER TABLE cases ADD COLUMN case_date TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(map_sql)?;
        }
        conn.execute(
            "UPDATE cases SET case_date = created_at WHERE case_date = ''",
            [],
        )
        .map_err(map_sql)?;
        // Index lives here (not in SCHEMA_SQL) because legacy DBs need the
        // ALTER TABLE above before the column is real.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS cases_case_date_idx ON cases(case_date DESC)",
            [],
        )
        .map_err(map_sql)?;
        Ok(())
    }

    /// Idempotent migration: add `patient_label` (non-null, default '') and
    /// `latest_error` (nullable) to the `cases` table when missing. Both
    /// are post-1.0 additions and existing rows would otherwise lack them.
    fn migrate_cases_patient_label_and_error(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(cases)").map_err(map_sql)?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(map_sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql)?;
        if !cols.iter().any(|c| c == "patient_label") {
            conn.execute(
                "ALTER TABLE cases ADD COLUMN patient_label TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(map_sql)?;
        }
        if !cols.iter().any(|c| c == "latest_error") {
            conn.execute("ALTER TABLE cases ADD COLUMN latest_error TEXT", [])
                .map_err(map_sql)?;
        }
        Ok(())
    }

    /// Idempotent migration: add privacy columns. Existing non-empty
    /// `original_text` rows are marked as `legacy_retained` and
    /// fingerprinted; empty rows become `discarded`.
    fn migrate_cases_privacy(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(cases)").map_err(map_sql)?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(map_sql)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sql)?;
        if !cols.iter().any(|c| c == "raw_text_sha256") {
            conn.execute(
                "ALTER TABLE cases ADD COLUMN raw_text_sha256 TEXT NOT NULL DEFAULT ''",
                [],
            )
            .map_err(map_sql)?;
        }
        if !cols.iter().any(|c| c == "raw_text_retention") {
            conn.execute(
                "ALTER TABLE cases ADD COLUMN raw_text_retention TEXT NOT NULL DEFAULT 'discarded'",
                [],
            )
            .map_err(map_sql)?;
        }

        let mut rows = conn
            .prepare("SELECT id, original_text, raw_text_sha256, raw_text_retention FROM cases")
            .map_err(map_sql)?;
        let rows = rows
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(map_sql)?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, original_text, stored_hash, retention) = row.map_err(map_sql)?;
            if !stored_hash.is_empty() {
                continue;
            }
            let hash = if original_text.is_empty() {
                String::new()
            } else {
                sha256_hex(original_text.as_bytes())
            };
            let next_retention = if !original_text.is_empty() && retention == "discarded" {
                RawTextRetention::LegacyRetained.as_db_str()
            } else {
                RawTextRetention::Discarded.as_db_str()
            };
            updates.push((id, hash, next_retention.to_owned()));
        }
        for (id, hash, retention) in updates {
            conn.execute(
                "UPDATE cases
                    SET raw_text_sha256 = ?1,
                        raw_text_retention = ?2
                  WHERE id = ?3",
                params![hash, retention, id],
            )
            .map_err(map_sql)?;
        }
        Ok(())
    }

    /// Path the connection was opened against.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Insert a case row.
    pub fn insert_case(&self, c: &CaseRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO cases
                   (id, created_at, workspace_id, question, original_text, masked_text,
                    deident_pipeline_id, status, case_date, patient_label, latest_error,
                    raw_text_sha256, raw_text_retention)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    c.id,
                    c.created_at.to_rfc3339(),
                    c.workspace_id,
                    c.question,
                    c.original_text,
                    c.masked_text,
                    c.deident_pipeline_id,
                    c.status.as_db_str(),
                    c.case_date.to_rfc3339(),
                    c.patient_label,
                    c.latest_error.as_deref(),
                    c.raw_text_sha256,
                    c.raw_text_retention.as_db_str(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Overwrite the editable fields of a draft case. Used when the
    /// clinician opens a draft from the list, adds clinical context, and
    /// fires "Run committee" — the values flow back through this UPDATE
    /// before the pipeline runs against the existing row.
    #[allow(clippy::too_many_arguments)]
    pub fn update_case_draft_content(
        &self,
        case_id: &str,
        original_text: &str,
        masked_text: &str,
        deident_pipeline_id: &str,
        question: &str,
        raw_text_sha256: &str,
        raw_text_retention: RawTextRetention,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE cases
                    SET original_text = ?1,
                        masked_text = ?2,
                        deident_pipeline_id = ?3,
                        question = ?4,
                        raw_text_sha256 = ?5,
                        raw_text_retention = ?6
                  WHERE id = ?7",
                params![
                    original_text,
                    masked_text,
                    deident_pipeline_id,
                    question,
                    raw_text_sha256,
                    raw_text_retention.as_db_str(),
                    case_id,
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Transition a case to a new status — used to promote a `Draft` to
    /// `Completed`/`Failed` once the pipeline has run against it. A
    /// successful transition (Completed) also clears any stale
    /// `latest_error` left behind by a previous failed attempt; Failed
    /// transitions leave it intact so `set_case_error` can populate it.
    pub fn mark_case_status(&self, case_id: &str, status: CaseStatus) -> Result<()> {
        if matches!(status, CaseStatus::ReviewReady | CaseStatus::Finalized) {
            self.conn
                .execute(
                    "UPDATE cases SET status = ?1, latest_error = NULL WHERE id = ?2",
                    params![status.as_db_str(), case_id],
                )
                .map_err(map_sql)?;
        } else {
            self.conn
                .execute(
                    "UPDATE cases SET status = ?1 WHERE id = ?2",
                    params![status.as_db_str(), case_id],
                )
                .map_err(map_sql)?;
        }
        Ok(())
    }

    /// Purge locally retained raw PHI for a case while preserving the
    /// de-identified narrative and SHA-256 fingerprint.
    pub fn purge_case_phi(&self, case_id: &str) -> Result<()> {
        let current: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT original_text, raw_text_sha256 FROM cases WHERE id = ?1",
                params![case_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(map_sql)?;
        let Some((original, stored_hash)) = current else {
            return Ok(());
        };
        let hash = if stored_hash.is_empty() && !original.is_empty() {
            sha256_hex(original.as_bytes())
        } else {
            stored_hash
        };
        self.conn
            .execute(
                "UPDATE cases
                    SET original_text = '',
                        raw_text_sha256 = ?1,
                        raw_text_retention = ?2
                  WHERE id = ?3",
                params![hash, RawTextRetention::Discarded.as_db_str(), case_id],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Persist (or clear) the latest error message for a case. Pass `None`
    /// to clear. Surfaced in the detail view when `status == Failed`.
    pub fn set_case_error(&self, case_id: &str, error: Option<&str>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE cases SET latest_error = ?1 WHERE id = ?2",
                params![error, case_id],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Update the `patient_label` of an existing case. Used when the draft
    /// is opened, edited, and the clinician adjusts the suggested label
    /// before running the committee.
    pub fn set_case_patient_label(&self, case_id: &str, label: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE cases SET patient_label = ?1 WHERE id = ?2",
                params![label, case_id],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert a verdict row.
    pub fn insert_verdict(&self, v: &VerdictRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO verdicts
                   (id, case_id, prompt_version, provider_id, model, latency_ms,
                    input_tokens, output_tokens, output_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    v.id,
                    v.case_id,
                    v.prompt_version,
                    v.provider_id,
                    v.model,
                    v.latency_ms as i64,
                    i64::from(v.input_tokens),
                    i64::from(v.output_tokens),
                    v.output_json,
                    v.created_at.to_rfc3339(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert a retrieval trace row.
    pub fn insert_trace(&self, t: &RetrievalTrace) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO retrieval_traces
                   (verdict_id, evidence_refs_json, past_cases_refs_json,
                    online_evidence_refs_json, attachment_refs_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    t.verdict_id,
                    serde_json::to_string(&t.evidence_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&t.past_cases_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&t.online_evidence_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&t.attachment_refs).unwrap_or_else(|_| "[]".into()),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert a case attachment row.
    pub fn insert_attachment(&self, a: &CaseAttachment) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO case_attachments
                   (id, case_id, position, original_filename, stored_path,
                    sha256, doc_type, mime, extracted_text, needs_ocr,
                    byte_size, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    a.id,
                    a.case_id,
                    i64::from(a.position),
                    a.original_filename,
                    a.stored_path,
                    a.sha256,
                    a.doc_type,
                    a.mime,
                    a.extracted_text,
                    i64::from(a.needs_ocr),
                    a.byte_size as i64,
                    a.created_at.to_rfc3339(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// List attachments for a case, ordered by `position`.
    pub fn list_attachments_for_case(&self, case_id: &str) -> Result<Vec<CaseAttachment>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, case_id, position, original_filename, stored_path,
                        sha256, doc_type, mime, extracted_text, needs_ocr,
                        byte_size, created_at
                 FROM case_attachments
                 WHERE case_id = ?1
                 ORDER BY position ASC",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map(params![case_id], row_to_attachment)
            .map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_sql)?);
        }
        Ok(out)
    }

    /// Delete attachments belonging to a case (used when a case is removed).
    pub fn delete_attachments_for_case(&self, case_id: &str) -> Result<usize> {
        self.conn
            .execute(
                "DELETE FROM case_attachments WHERE case_id = ?1",
                params![case_id],
            )
            .map_err(map_sql)
    }

    /// Remove raw attachment files for a case while keeping the de-identified
    /// extracted text, filename, hash and `[A*]` trace rows.
    pub fn purge_case_attachment_files(&self, case_id: &str) -> Result<usize> {
        let attachments = self.list_attachments_for_case(case_id)?;
        let mut purged = 0usize;
        for att in attachments {
            if att.stored_path.trim().is_empty() {
                continue;
            }
            match std::fs::remove_file(&att.stored_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %att.stored_path,
                        "could not remove raw attachment file"
                    );
                }
            }
            self.conn
                .execute(
                    "UPDATE case_attachments
                        SET stored_path = '',
                            byte_size = 0,
                            needs_ocr = 0
                      WHERE id = ?1",
                    params![att.id],
                )
                .map_err(map_sql)?;
            purged += 1;
        }
        Ok(purged)
    }

    /// Insert a deliberation trace row.
    pub fn insert_deliberation_trace(&self, t: &DeliberationTrace) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO deliberation_traces
                   (id, verdict_id, briefing_output, drafting_output, redteam_output,
                    total_input_tokens, total_output_tokens, duration_ms, vision_used,
                    created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    t.id,
                    t.verdict_id,
                    t.briefing_output,
                    t.drafting_output,
                    t.redteam_output,
                    i64::from(t.total_input_tokens),
                    i64::from(t.total_output_tokens),
                    t.duration_ms as i64,
                    i64::from(t.vision_used),
                    t.created_at.to_rfc3339(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Fetch the most recent deliberation trace for a given verdict.
    pub fn get_deliberation_trace(&self, verdict_id: &str) -> Result<Option<DeliberationTrace>> {
        self.conn
            .query_row(
                "SELECT id, verdict_id, briefing_output, drafting_output, redteam_output,
                        total_input_tokens, total_output_tokens, duration_ms, vision_used,
                        created_at
                 FROM deliberation_traces
                 WHERE verdict_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![verdict_id],
                row_to_deliberation_trace,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Upsert a feedback row.
    pub fn upsert_feedback(&self, f: &FeedbackRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO feedback (case_id, kind, reason, modified_verdict_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(case_id) DO UPDATE SET
                   kind = excluded.kind,
                   reason = excluded.reason,
                   modified_verdict_json = excluded.modified_verdict_json,
                   created_at = excluded.created_at",
                params![
                    f.case_id,
                    f.kind.as_db_str(),
                    f.reason,
                    f.modified_verdict_json,
                    f.created_at.to_rfc3339(),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Insert or update explicit clinician review metadata and mark the
    /// case finalized.
    pub fn finalize_review(&self, review: &ReviewMetadataRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO review_metadata
                   (case_id, verdict_id, decision, reviewer_name, reviewer_role, note,
                    final_verdict_json, diff_summary, reviewed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(case_id) DO UPDATE SET
                   verdict_id = excluded.verdict_id,
                   decision = excluded.decision,
                   reviewer_name = excluded.reviewer_name,
                   reviewer_role = excluded.reviewer_role,
                   note = excluded.note,
                   final_verdict_json = excluded.final_verdict_json,
                   diff_summary = excluded.diff_summary,
                   reviewed_at = excluded.reviewed_at",
                params![
                    review.case_id,
                    review.verdict_id,
                    review.decision.as_db_str(),
                    review.reviewer_name,
                    review.reviewer_role,
                    review.note,
                    review.final_verdict_json,
                    review.diff_summary,
                    review.reviewed_at.to_rfc3339(),
                ],
            )
            .map_err(map_sql)?;
        self.mark_case_status(&review.case_id, CaseStatus::Finalized)?;
        self.purge_case_phi(&review.case_id)?;
        Ok(())
    }

    pub fn get_review_metadata(&self, case_id: &str) -> Result<Option<ReviewMetadataRecord>> {
        self.conn
            .query_row(
                "SELECT case_id, verdict_id, decision, reviewer_name, reviewer_role, note,
                        final_verdict_json, diff_summary, reviewed_at
                 FROM review_metadata
                 WHERE case_id = ?1",
                params![case_id],
                row_to_review_metadata,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Insert one audit run. Payload mode is stored as metadata; the run
    /// itself stores fingerprints by default.
    pub fn insert_audit_run(&self, run: &AuditRunRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO audit_runs
                   (id, case_id, verdict_id, provider_id, model, data_boundary_mode,
                    payload_mode, active_skill_id, started_at, completed_at, latency_ms,
                    input_tokens, output_tokens, prompt_sha256, output_sha256,
                    evidence_refs_json, past_cases_refs_json, online_evidence_refs_json,
                    attachment_refs_json, raw_text_retention, status, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    run.id,
                    run.case_id,
                    run.verdict_id,
                    run.provider_id,
                    run.model,
                    run.data_boundary_mode.as_db_str(),
                    run.payload_mode.as_db_str(),
                    run.active_skill_id,
                    run.started_at.to_rfc3339(),
                    run.completed_at.map(|d| d.to_rfc3339()),
                    run.latency_ms as i64,
                    i64::from(run.input_tokens),
                    i64::from(run.output_tokens),
                    run.prompt_sha256,
                    run.output_sha256,
                    serde_json::to_string(&run.evidence_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&run.past_cases_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&run.online_evidence_refs)
                        .unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&run.attachment_refs).unwrap_or_else(|_| "[]".into()),
                    run.raw_text_retention.as_db_str(),
                    run.status,
                    run.error,
                ],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    pub fn list_audit_runs(&self, limit: usize) -> Result<Vec<AuditRunRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, case_id, verdict_id, provider_id, model, data_boundary_mode,
                        payload_mode, active_skill_id, started_at, completed_at, latency_ms,
                        input_tokens, output_tokens, prompt_sha256, output_sha256,
                        evidence_refs_json, past_cases_refs_json, online_evidence_refs_json,
                        attachment_refs_json, raw_text_retention, status, error
                 FROM audit_runs
                 ORDER BY started_at DESC
                 LIMIT ?1",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_audit_run)
            .map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_sql)?);
        }
        Ok(out)
    }

    pub fn latest_audit_for_case(&self, case_id: &str) -> Result<Option<AuditRunRecord>> {
        self.conn
            .query_row(
                "SELECT id, case_id, verdict_id, provider_id, model, data_boundary_mode,
                        payload_mode, active_skill_id, started_at, completed_at, latency_ms,
                        input_tokens, output_tokens, prompt_sha256, output_sha256,
                        evidence_refs_json, past_cases_refs_json, online_evidence_refs_json,
                        attachment_refs_json, raw_text_retention, status, error
                 FROM audit_runs
                 WHERE case_id = ?1
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![case_id],
                row_to_audit_run,
            )
            .optional()
            .map_err(map_sql)
    }

    pub fn audit_status(&self) -> Result<AuditStatus> {
        let run_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM audit_runs", [], |r| r.get(0))
            .map_err(map_sql)?;
        let retained_raw_cases: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM cases
                 WHERE original_text != '' AND raw_text_retention != 'discarded'",
                [],
                |r| r.get(0),
            )
            .map_err(map_sql)?;
        let legacy_retained_cases: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM cases WHERE raw_text_retention = 'legacy_retained'",
                [],
                |r| r.get(0),
            )
            .map_err(map_sql)?;
        Ok(AuditStatus {
            run_count: run_count.max(0) as u64,
            payload_mode: AuditPayloadMode::Fingerprint,
            retained_raw_cases: retained_raw_cases.max(0) as u64,
            legacy_retained_cases: legacy_retained_cases.max(0) as u64,
        })
    }

    pub fn cleanup_discarded_phi(&self) -> Result<usize> {
        self.conn
            .execute(
                "UPDATE cases
                    SET original_text = ''
                  WHERE raw_text_retention = 'discarded' AND original_text != ''",
                [],
            )
            .map_err(map_sql)
    }

    /// List cases ordered by `case_date` (user-facing date) descending.
    /// Falls back to `created_at` on ties for stable order.
    pub fn list_cases(&self, limit: usize) -> Result<Vec<CaseRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, workspace_id, question, original_text, masked_text,
                        deident_pipeline_id, status, case_date, patient_label, latest_error,
                        raw_text_sha256, raw_text_retention
                 FROM cases
                 ORDER BY case_date DESC, created_at DESC
                 LIMIT ?1",
            )
            .map_err(map_sql)?;
        let mut out = Vec::new();
        let rows = stmt
            .query_map(params![limit as i64], row_to_case)
            .map_err(map_sql)?;
        for row in rows {
            out.push(row.map_err(map_sql)?);
        }
        Ok(out)
    }

    /// Fetch a case by id.
    pub fn get_case(&self, id: &str) -> Result<Option<CaseRecord>> {
        self.conn
            .query_row(
                "SELECT id, created_at, workspace_id, question, original_text, masked_text,
                        deident_pipeline_id, status, case_date, patient_label, latest_error,
                        raw_text_sha256, raw_text_retention
                 FROM cases WHERE id = ?1",
                params![id],
                row_to_case,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Bulk-update the `case_date` of one or many cases. Uses a single
    /// transaction so partial failures roll back. N is expected to stay
    /// small (UI typically operates on ≤ 50 ids).
    pub fn update_case_date(&mut self, ids: &[String], new_date: DateTime<Utc>) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let new_date_str = new_date.to_rfc3339();
        let tx = self.conn.transaction().map_err(map_sql)?;
        {
            let mut stmt = tx
                .prepare("UPDATE cases SET case_date = ?1 WHERE id = ?2")
                .map_err(map_sql)?;
            for id in ids {
                stmt.execute(params![new_date_str, id]).map_err(map_sql)?;
            }
        }
        tx.commit().map_err(map_sql)?;
        Ok(())
    }

    /// Delete one or many cases by id. Relies on `ON DELETE CASCADE`
    /// (foreign_keys=ON, set at connection open) to clean up
    /// verdicts, retrieval_traces, feedback, case_memory,
    /// case_attachments and deliberation_traces in the same
    /// transaction. Returns the number of `cases` rows actually removed
    /// (ids that no longer exist are silently skipped).
    ///
    /// On-disk files (the `cases/<id>/` directory with attachments) are
    /// the caller's responsibility — this only touches SQLite.
    pub fn delete_cases(&mut self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction().map_err(map_sql)?;
        let mut deleted = 0usize;
        {
            let mut stmt = tx
                .prepare("DELETE FROM cases WHERE id = ?1")
                .map_err(map_sql)?;
            for id in ids {
                deleted += stmt.execute(params![id]).map_err(map_sql)?;
            }
        }
        tx.commit().map_err(map_sql)?;
        Ok(deleted)
    }

    /// Latest verdict for a given case.
    pub fn latest_verdict(&self, case_id: &str) -> Result<Option<VerdictRecord>> {
        self.conn
            .query_row(
                "SELECT id, case_id, prompt_version, provider_id, model, latency_ms,
                        input_tokens, output_tokens, output_json, created_at
                 FROM verdicts
                 WHERE case_id = ?1
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![case_id],
                row_to_verdict,
            )
            .optional()
            .map_err(map_sql)
    }

    /// Latest feedback for a case.
    pub fn get_feedback(&self, case_id: &str) -> Result<Option<FeedbackRecord>> {
        self.conn
            .query_row(
                "SELECT case_id, kind, reason, modified_verdict_json, created_at
                 FROM feedback WHERE case_id = ?1",
                params![case_id],
                row_to_feedback,
            )
            .optional()
            .map_err(map_sql)
    }

    // ----- case memory (Phase 5) ---------------------------------------

    /// Upsert a case-memory entry with its embedding.
    pub fn upsert_case_memory(
        &self,
        case_id: &str,
        embedding: &[f32],
        case_summary: &str,
        verdict_summary: &str,
    ) -> Result<()> {
        let blob = vec_to_bytes(embedding);
        self.conn
            .execute(
                "INSERT INTO case_memory (case_id, embedding, case_summary, verdict_summary)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(case_id) DO UPDATE SET
                   embedding = excluded.embedding,
                   case_summary = excluded.case_summary,
                   verdict_summary = excluded.verdict_summary",
                params![case_id, blob, case_summary, verdict_summary],
            )
            .map_err(map_sql)?;
        Ok(())
    }

    /// Find the top-K past cases most similar to `query`, filtered by a
    /// minimum cosine similarity. Results are sorted by similarity desc.
    pub fn similar_past_cases(
        &self,
        query: &[f32],
        k: usize,
        min_similarity: f32,
    ) -> Result<Vec<PastCaseHit>> {
        if query.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let query_norm = vec_norm(query);
        if query_norm == 0.0 {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT m.case_id, m.embedding, m.case_summary, m.verdict_summary,
                        f.kind, f.reason
                 FROM case_memory m
                 LEFT JOIN feedback f ON f.case_id = m.case_id",
            )
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([], |r| {
                let case_id: String = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                let case_summary: String = r.get(2)?;
                let verdict_summary: String = r.get(3)?;
                let feedback_kind: Option<String> = r.get(4)?;
                let feedback_reason: Option<String> = r.get(5)?;
                Ok((
                    case_id,
                    blob,
                    case_summary,
                    verdict_summary,
                    feedback_kind,
                    feedback_reason,
                ))
            })
            .map_err(map_sql)?;
        let mut all = Vec::new();
        for row in rows {
            let (case_id, blob, case_summary, verdict_summary, feedback_kind, feedback_reason) =
                row.map_err(map_sql)?;
            let embedding = bytes_to_vec(&blob);
            if embedding.len() != query.len() {
                // dim drift — skip.
                continue;
            }
            let sim = cosine_norm(query, &embedding, query_norm);
            if sim < min_similarity {
                continue;
            }
            all.push(PastCaseHit {
                case_id,
                case_summary,
                verdict_summary,
                feedback_kind: feedback_kind.as_deref().and_then(FeedbackKind::from_db_str),
                feedback_reason,
                similarity: sim,
            });
        }
        all.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.truncate(k);
        Ok(all)
    }

    // ----- stats + export (Phase 5) ------------------------------------

    /// Aggregate counters for the `stats` CLI.
    pub fn stats(&self) -> Result<StoreStats> {
        let total_cases: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM cases", [], |r| r.get(0))
            .map_err(map_sql)?;
        let completed: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM cases WHERE status IN ('review_ready', 'finalized', 'finalized_legacy', 'completed')",
                [],
                |r| r.get(0),
            )
            .map_err(map_sql)?;
        let failed: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM cases WHERE status = 'failed'",
                [],
                |r| r.get(0),
            )
            .map_err(map_sql)?;

        let mut feedback_counts = std::collections::BTreeMap::new();
        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM feedback GROUP BY kind")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([], |r| {
                let kind: String = r.get(0)?;
                let n: i64 = r.get(1)?;
                Ok((kind, n))
            })
            .map_err(map_sql)?;
        for row in rows {
            let (kind, n) = row.map_err(map_sql)?;
            feedback_counts.insert(kind, n as u64);
        }

        let avg_latency: Option<f64> = self
            .conn
            .query_row("SELECT AVG(latency_ms) FROM verdicts", [], |r| r.get(0))
            .optional()
            .map_err(map_sql)?
            .flatten();
        let recent_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM cases
                 WHERE julianday('now') - julianday(created_at) <= 7",
                [],
                |r| r.get(0),
            )
            .map_err(map_sql)?;

        Ok(StoreStats {
            total_cases: total_cases.max(0) as u64,
            completed: completed.max(0) as u64,
            failed: failed.max(0) as u64,
            feedback_counts,
            avg_latency_ms: avg_latency,
            cases_last_7d: recent_count.max(0) as u64,
        })
    }

    /// JSON-friendly dump of every case + latest verdict + feedback.
    /// Uses **masked** case text only; the original is never exported.
    pub fn export(&self) -> Result<Vec<ExportedCase>> {
        let cases = self.list_cases(usize::MAX)?;
        let mut out = Vec::with_capacity(cases.len());
        for c in cases {
            let verdict = self.latest_verdict(&c.id)?;
            let feedback = self.get_feedback(&c.id)?;
            out.push(ExportedCase {
                case_id: c.id,
                created_at: c.created_at,
                case_date: c.case_date,
                workspace_id: c.workspace_id,
                question: c.question,
                masked_text: c.masked_text,
                deident_pipeline_id: c.deident_pipeline_id,
                status: c.status,
                raw_text_sha256: c.raw_text_sha256,
                raw_text_retention: c.raw_text_retention,
                verdict_json: verdict.map(|v| v.output_json),
                feedback: feedback.map(|f| ExportedFeedback {
                    kind: f.kind,
                    reason: f.reason,
                    modified_verdict_json: f.modified_verdict_json,
                    created_at: f.created_at,
                }),
            });
        }
        Ok(out)
    }
}

/// One similar-case hit returned by [`CaseStore::similar_past_cases`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PastCaseHit {
    pub case_id: String,
    pub case_summary: String,
    pub verdict_summary: String,
    pub feedback_kind: Option<FeedbackKind>,
    pub feedback_reason: Option<String>,
    pub similarity: f32,
}

/// Aggregate counters surfaced by the `stats` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub total_cases: u64,
    pub completed: u64,
    pub failed: u64,
    pub feedback_counts: std::collections::BTreeMap<String, u64>,
    pub avg_latency_ms: Option<f64>,
    pub cases_last_7d: u64,
}

/// Row in the JSON export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedCase {
    pub case_id: String,
    pub created_at: DateTime<Utc>,
    pub case_date: DateTime<Utc>,
    pub workspace_id: String,
    pub question: String,
    pub masked_text: String,
    pub deident_pipeline_id: String,
    pub status: CaseStatus,
    pub raw_text_sha256: String,
    pub raw_text_retention: RawTextRetention,
    pub verdict_json: Option<String>,
    pub feedback: Option<ExportedFeedback>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFeedback {
    pub kind: FeedbackKind,
    pub reason: Option<String>,
    pub modified_verdict_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

fn bytes_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn vec_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn cosine_norm(a: &[f32], b: &[f32], norm_a: f32) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_b = vec_norm(b);
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn row_to_case(r: &rusqlite::Row<'_>) -> rusqlite::Result<CaseRecord> {
    let created_at = parse_dt(&r.get::<_, String>(1)?);
    let case_date_str: String = r.get(8)?;
    // Defensive: blank `case_date` shouldn't happen post-migration, but
    // fall back to `created_at` rather than `now()` so stale rows don't
    // appear to time-travel.
    let case_date = if case_date_str.is_empty() {
        created_at
    } else {
        parse_dt(&case_date_str)
    };
    Ok(CaseRecord {
        id: r.get(0)?,
        created_at,
        case_date,
        workspace_id: r.get(2)?,
        question: r.get(3)?,
        original_text: r.get(4)?,
        masked_text: r.get(5)?,
        deident_pipeline_id: r.get(6)?,
        status: CaseStatus::from_db_str(&r.get::<_, String>(7)?).unwrap_or(CaseStatus::Failed),
        patient_label: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        latest_error: r.get::<_, Option<String>>(10)?,
        raw_text_sha256: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
        raw_text_retention: RawTextRetention::from_db_str(
            &r.get::<_, Option<String>>(12)?
                .unwrap_or_else(|| "legacy_retained".into()),
        ),
    })
}

fn row_to_review_metadata(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewMetadataRecord> {
    Ok(ReviewMetadataRecord {
        case_id: r.get(0)?,
        verdict_id: r.get(1)?,
        decision: ReviewDecision::from_db_str(&r.get::<_, String>(2)?)
            .unwrap_or(ReviewDecision::Accept),
        reviewer_name: r.get(3)?,
        reviewer_role: r.get(4)?,
        note: r.get(5)?,
        final_verdict_json: r.get(6)?,
        diff_summary: r.get(7)?,
        reviewed_at: parse_dt(&r.get::<_, String>(8)?),
    })
}

fn row_to_audit_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRunRecord> {
    let refs = |idx: usize| -> rusqlite::Result<Vec<String>> {
        let raw: String = r.get(idx)?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    };
    Ok(AuditRunRecord {
        id: r.get(0)?,
        case_id: r.get(1)?,
        verdict_id: r.get(2)?,
        provider_id: r.get(3)?,
        model: r.get(4)?,
        data_boundary_mode: DataBoundaryMode::from_db_str(&r.get::<_, String>(5)?),
        payload_mode: AuditPayloadMode::from_db_str(&r.get::<_, String>(6)?),
        active_skill_id: r.get(7)?,
        started_at: parse_dt(&r.get::<_, String>(8)?),
        completed_at: r.get::<_, Option<String>>(9)?.map(|s| parse_dt(&s)),
        latency_ms: r.get::<_, i64>(10)?.max(0) as u64,
        input_tokens: u32::try_from(r.get::<_, i64>(11)?.max(0)).unwrap_or(0),
        output_tokens: u32::try_from(r.get::<_, i64>(12)?.max(0)).unwrap_or(0),
        prompt_sha256: r.get(13)?,
        output_sha256: r.get(14)?,
        evidence_refs: refs(15)?,
        past_cases_refs: refs(16)?,
        online_evidence_refs: refs(17)?,
        attachment_refs: refs(18)?,
        raw_text_retention: RawTextRetention::from_db_str(&r.get::<_, String>(19)?),
        status: r.get(20)?,
        error: r.get(21)?,
    })
}

fn row_to_verdict(r: &rusqlite::Row<'_>) -> rusqlite::Result<VerdictRecord> {
    Ok(VerdictRecord {
        id: r.get(0)?,
        case_id: r.get(1)?,
        prompt_version: r.get(2)?,
        provider_id: r.get(3)?,
        model: r.get(4)?,
        latency_ms: r.get::<_, i64>(5)?.max(0) as u64,
        input_tokens: u32::try_from(r.get::<_, i64>(6)?.max(0)).unwrap_or(u32::MAX),
        output_tokens: u32::try_from(r.get::<_, i64>(7)?.max(0)).unwrap_or(u32::MAX),
        output_json: r.get(8)?,
        created_at: parse_dt(&r.get::<_, String>(9)?),
    })
}

fn row_to_deliberation_trace(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeliberationTrace> {
    Ok(DeliberationTrace {
        id: r.get(0)?,
        verdict_id: r.get(1)?,
        briefing_output: r.get(2)?,
        drafting_output: r.get(3)?,
        redteam_output: r.get(4)?,
        total_input_tokens: u32::try_from(r.get::<_, i64>(5)?.max(0)).unwrap_or(0),
        total_output_tokens: u32::try_from(r.get::<_, i64>(6)?.max(0)).unwrap_or(0),
        duration_ms: r.get::<_, i64>(7)?.max(0) as u64,
        vision_used: r.get::<_, i64>(8)? != 0,
        created_at: parse_dt(&r.get::<_, String>(9)?),
    })
}

fn row_to_attachment(r: &rusqlite::Row<'_>) -> rusqlite::Result<CaseAttachment> {
    Ok(CaseAttachment {
        id: r.get(0)?,
        case_id: r.get(1)?,
        position: u32::try_from(r.get::<_, i64>(2)?.max(0)).unwrap_or(0),
        original_filename: r.get(3)?,
        stored_path: r.get(4)?,
        sha256: r.get(5)?,
        doc_type: r.get(6)?,
        mime: r.get(7)?,
        extracted_text: r.get(8)?,
        needs_ocr: r.get::<_, i64>(9)? != 0,
        byte_size: r.get::<_, i64>(10)?.max(0) as u64,
        created_at: parse_dt(&r.get::<_, String>(11)?),
    })
}

fn row_to_feedback(r: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackRecord> {
    Ok(FeedbackRecord {
        case_id: r.get(0)?,
        kind: FeedbackKind::from_db_str(&r.get::<_, String>(1)?).unwrap_or(FeedbackKind::Accept),
        reason: r.get(2)?,
        modified_verdict_json: r.get(3)?,
        created_at: parse_dt(&r.get::<_, String>(4)?),
    })
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn map_sql(e: rusqlite::Error) -> Error {
    Error::Rag(format!("verdict sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_case(id: &str) -> CaseRecord {
        let now = Utc::now();
        CaseRecord {
            id: id.into(),
            created_at: now,
            case_date: now,
            workspace_id: "ws".into(),
            question: "q".into(),
            original_text: "o".into(),
            masked_text: "m".into(),
            deident_pipeline_id: "p".into(),
            status: CaseStatus::ReviewReady,
            patient_label: String::new(),
            latest_error: None,
            raw_text_sha256: sha256_hex("o"),
            raw_text_retention: RawTextRetention::TemporaryDraft,
        }
    }

    fn sample_verdict(case_id: &str) -> VerdictRecord {
        VerdictRecord {
            id: format!("{case_id}-v1"),
            case_id: case_id.into(),
            prompt_version: "v1".into(),
            provider_id: "mock".into(),
            model: "mock-model".into(),
            latency_ms: 12,
            input_tokens: 1,
            output_tokens: 1,
            output_json: "{}".into(),
            created_at: Utc::now(),
        }
    }

    fn sample_audit(case_id: &str) -> AuditRunRecord {
        AuditRunRecord {
            id: format!("{case_id}-run-1"),
            case_id: case_id.into(),
            verdict_id: Some(format!("{case_id}-v1")),
            provider_id: "mock".into(),
            model: "mock-model".into(),
            data_boundary_mode: DataBoundaryMode::DeidCloud,
            payload_mode: AuditPayloadMode::Fingerprint,
            active_skill_id: Some("tumor-board".into()),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            latency_ms: 12,
            input_tokens: 3,
            output_tokens: 5,
            prompt_sha256: sha256_hex("prompt"),
            output_sha256: sha256_hex("output"),
            evidence_refs: vec!["E1".into()],
            past_cases_refs: vec!["P1".into()],
            online_evidence_refs: vec!["X1".into()],
            attachment_refs: vec!["A1".into()],
            raw_text_retention: RawTextRetention::Discarded,
            status: "ok".into(),
            error: None,
        }
    }

    #[test]
    fn round_trip_case_verdict_feedback() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("c1")).unwrap();
        store.insert_verdict(&sample_verdict("c1")).unwrap();
        store
            .insert_trace(&RetrievalTrace {
                verdict_id: "c1-v1".into(),
                evidence_refs: vec!["E1".into()],
                past_cases_refs: vec![],
                online_evidence_refs: vec![],
                attachment_refs: vec![],
            })
            .unwrap();
        store
            .upsert_feedback(&FeedbackRecord {
                case_id: "c1".into(),
                kind: FeedbackKind::Accept,
                reason: Some("looks right".into()),
                modified_verdict_json: None,
                created_at: Utc::now(),
            })
            .unwrap();

        let listed = store.list_cases(10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "c1");

        let v = store.latest_verdict("c1").unwrap().unwrap();
        assert_eq!(v.id, "c1-v1");

        let f = store.get_feedback("c1").unwrap().unwrap();
        assert_eq!(f.kind, FeedbackKind::Accept);
    }

    #[test]
    fn migration_marks_legacy_raw_text_retained() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("cases.sqlite");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE cases (
                    id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    question TEXT NOT NULL,
                    original_text TEXT NOT NULL,
                    masked_text TEXT NOT NULL,
                    deident_pipeline_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    case_date TEXT NOT NULL DEFAULT '',
                    patient_label TEXT NOT NULL DEFAULT '',
                    latest_error TEXT
                );
                INSERT INTO cases
                    (id, created_at, workspace_id, question, original_text, masked_text,
                     deident_pipeline_id, status, case_date, patient_label, latest_error)
                VALUES
                    ('legacy','2026-01-01T00:00:00Z','ws','q','Paciente Ana','Paciente [NAME]',
                     'p','completed','2026-01-01T00:00:00Z','','');",
            )
            .unwrap();
        }

        let store = CaseStore::open(&db).unwrap();
        let fetched = store.get_case("legacy").unwrap().unwrap();
        assert_eq!(fetched.status, CaseStatus::FinalizedLegacy);
        assert_eq!(fetched.raw_text_sha256, sha256_hex("Paciente Ana"));
        assert_eq!(fetched.raw_text_retention, RawTextRetention::LegacyRetained);
    }

    #[test]
    fn purge_case_phi_preserves_hash_and_deidentified_text() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("c-phi")).unwrap();

        store.purge_case_phi("c-phi").unwrap();
        let fetched = store.get_case("c-phi").unwrap().unwrap();
        assert_eq!(fetched.original_text, "");
        assert_eq!(fetched.masked_text, "m");
        assert_eq!(fetched.raw_text_sha256, sha256_hex("o"));
        assert_eq!(fetched.raw_text_retention, RawTextRetention::Discarded);
    }

    #[test]
    fn audit_run_round_trip_and_status_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("c-audit")).unwrap();
        store.insert_verdict(&sample_verdict("c-audit")).unwrap();
        store.insert_audit_run(&sample_audit("c-audit")).unwrap();

        let status = store.audit_status().unwrap();
        assert_eq!(status.run_count, 1);
        assert_eq!(status.payload_mode, AuditPayloadMode::Fingerprint);
        assert_eq!(status.retained_raw_cases, 1);

        let latest = store.latest_audit_for_case("c-audit").unwrap().unwrap();
        assert_eq!(latest.payload_mode, AuditPayloadMode::Fingerprint);
        assert_eq!(latest.evidence_refs, vec!["E1"]);
        assert_eq!(latest.past_cases_refs, vec!["P1"]);
        assert_eq!(latest.online_evidence_refs, vec!["X1"]);
        assert_eq!(latest.attachment_refs, vec!["A1"]);
        assert_eq!(latest.prompt_sha256, sha256_hex("prompt"));
    }

    #[test]
    fn finalize_review_marks_finalized_and_purges_phi() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("c-review")).unwrap();
        store.insert_verdict(&sample_verdict("c-review")).unwrap();

        store
            .finalize_review(&ReviewMetadataRecord {
                case_id: "c-review".into(),
                verdict_id: "c-review-v1".into(),
                decision: ReviewDecision::Accept,
                reviewer_name: Some("Dr Test".into()),
                reviewer_role: Some("oncology".into()),
                note: Some("reviewed".into()),
                final_verdict_json: Some("{\"ok\":true}".into()),
                diff_summary: None,
                reviewed_at: Utc::now(),
            })
            .unwrap();

        let fetched = store.get_case("c-review").unwrap().unwrap();
        assert_eq!(fetched.status, CaseStatus::Finalized);
        assert_eq!(fetched.original_text, "");
        assert_eq!(fetched.raw_text_retention, RawTextRetention::Discarded);

        let review = store.get_review_metadata("c-review").unwrap().unwrap();
        assert_eq!(review.decision, ReviewDecision::Accept);
        assert_eq!(review.reviewer_name.as_deref(), Some("Dr Test"));
    }

    #[test]
    fn set_case_error_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        let mut c = sample_case("c-err");
        c.status = CaseStatus::Draft;
        store.insert_case(&c).unwrap();
        // Initially the column is NULL.
        let fetched = store.get_case("c-err").unwrap().unwrap();
        assert_eq!(fetched.latest_error, None);

        // Marking Failed leaves latest_error alone — caller fills it.
        store.mark_case_status("c-err", CaseStatus::Failed).unwrap();
        store
            .set_case_error("c-err", Some("provider returned 400"))
            .unwrap();
        let fetched = store.get_case("c-err").unwrap().unwrap();
        assert_eq!(fetched.status, CaseStatus::Failed);
        assert_eq!(
            fetched.latest_error.as_deref(),
            Some("provider returned 400")
        );

        // Promoting to review-ready clears the stale error.
        store
            .mark_case_status("c-err", CaseStatus::ReviewReady)
            .unwrap();
        let fetched = store.get_case("c-err").unwrap().unwrap();
        assert_eq!(fetched.status, CaseStatus::ReviewReady);
        assert_eq!(fetched.latest_error, None);
    }

    #[test]
    fn patient_label_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        let mut c = sample_case("c-pl");
        c.patient_label = "Juan Pérez".into();
        store.insert_case(&c).unwrap();
        let fetched = store.get_case("c-pl").unwrap().unwrap();
        assert_eq!(fetched.patient_label, "Juan Pérez");

        store.set_case_patient_label("c-pl", "CR-IA-011").unwrap();
        let fetched = store.get_case("c-pl").unwrap().unwrap();
        assert_eq!(fetched.patient_label, "CR-IA-011");
    }

    #[test]
    fn migration_adds_patient_label_and_latest_error_to_legacy_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("cases.sqlite");
        // Hand-craft a legacy schema that's missing the new columns. Mirror
        // the production schema's NOT NULL constraints so the migration has
        // realistic work to do.
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE cases (
                    id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    question TEXT NOT NULL,
                    original_text TEXT NOT NULL,
                    masked_text TEXT NOT NULL,
                    deident_pipeline_id TEXT NOT NULL,
                    status TEXT NOT NULL
                );
                INSERT INTO cases VALUES ('old1','2026-01-01T00:00:00Z','ws','q','o','m','p','failed');",
            )
            .unwrap();
        }
        // Opening via CaseStore should add both columns idempotently.
        let store = CaseStore::open(&db).unwrap();
        let fetched = store.get_case("old1").unwrap().unwrap();
        assert_eq!(fetched.patient_label, "");
        assert_eq!(fetched.latest_error, None);
        // And `set_case_error` should now work on the legacy row.
        store.set_case_error("old1", Some("backfilled")).unwrap();
        let fetched = store.get_case("old1").unwrap().unwrap();
        assert_eq!(fetched.latest_error.as_deref(), Some("backfilled"));
    }

    #[test]
    fn attachments_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("cA")).unwrap();
        let att = CaseAttachment {
            id: "att-1".into(),
            case_id: "cA".into(),
            position: 1,
            original_filename: "labs.pdf".into(),
            stored_path: "/tmp/labs.pdf".into(),
            sha256: "abc".into(),
            doc_type: "pdf".into(),
            mime: "application/pdf".into(),
            extracted_text: "Hb 12.4".into(),
            needs_ocr: false,
            byte_size: 1024,
            created_at: Utc::now(),
        };
        store.insert_attachment(&att).unwrap();
        let img = CaseAttachment {
            id: "att-2".into(),
            case_id: "cA".into(),
            position: 2,
            original_filename: "ecg.png".into(),
            stored_path: "/tmp/ecg.png".into(),
            sha256: "def".into(),
            doc_type: "image".into(),
            mime: "image/png".into(),
            extracted_text: String::new(),
            needs_ocr: true,
            byte_size: 4096,
            created_at: Utc::now(),
        };
        store.insert_attachment(&img).unwrap();

        let list = store.list_attachments_for_case("cA").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].position, 1);
        assert_eq!(list[0].original_filename, "labs.pdf");
        assert_eq!(list[1].position, 2);
        assert!(list[1].needs_ocr);
        assert!(list[1].extracted_text.is_empty());

        let removed = store.delete_attachments_for_case("cA").unwrap();
        assert_eq!(removed, 2);
        assert!(store.list_attachments_for_case("cA").unwrap().is_empty());
    }

    #[test]
    fn purge_case_attachment_files_keeps_deidentified_text() {
        let tmp = tempfile::tempdir().unwrap();
        let attachment_path = tmp.path().join("labs.pdf");
        std::fs::write(&attachment_path, b"raw bytes").unwrap();
        let store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();
        store.insert_case(&sample_case("c-att-purge")).unwrap();
        store
            .insert_attachment(&CaseAttachment {
                id: "att-purge".into(),
                case_id: "c-att-purge".into(),
                position: 1,
                original_filename: "labs.pdf".into(),
                stored_path: attachment_path.display().to_string(),
                sha256: "abc".into(),
                doc_type: "pdf".into(),
                mime: "application/pdf".into(),
                extracted_text: "Hb 12.4".into(),
                needs_ocr: false,
                byte_size: 9,
                created_at: Utc::now(),
            })
            .unwrap();

        let purged = store.purge_case_attachment_files("c-att-purge").unwrap();
        assert_eq!(purged, 1);
        assert!(!attachment_path.exists());
        let atts = store.list_attachments_for_case("c-att-purge").unwrap();
        assert_eq!(atts[0].stored_path, "");
        assert_eq!(atts[0].byte_size, 0);
        assert_eq!(atts[0].extracted_text, "Hb 12.4");
        assert_eq!(atts[0].sha256, "abc");
    }

    #[test]
    fn delete_cases_cascades_to_children() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = CaseStore::open(tmp.path().join("cases.sqlite")).unwrap();

        // Two cases, only c1 gets verdict/feedback/attachment baggage —
        // we want to confirm cascade really fires and c2 is untouched.
        store.insert_case(&sample_case("c1")).unwrap();
        store.insert_case(&sample_case("c2")).unwrap();
        store.insert_verdict(&sample_verdict("c1")).unwrap();
        store
            .insert_trace(&RetrievalTrace {
                verdict_id: "c1-v1".into(),
                evidence_refs: vec!["E1".into()],
                past_cases_refs: vec![],
                online_evidence_refs: vec![],
                attachment_refs: vec![],
            })
            .unwrap();
        store
            .upsert_feedback(&FeedbackRecord {
                case_id: "c1".into(),
                kind: FeedbackKind::Accept,
                reason: None,
                modified_verdict_json: None,
                created_at: Utc::now(),
            })
            .unwrap();
        store
            .insert_attachment(&CaseAttachment {
                id: "att-x".into(),
                case_id: "c1".into(),
                position: 1,
                original_filename: "labs.pdf".into(),
                stored_path: "/tmp/labs.pdf".into(),
                sha256: "abc".into(),
                doc_type: "pdf".into(),
                mime: "application/pdf".into(),
                extracted_text: "x".into(),
                needs_ocr: false,
                byte_size: 1,
                created_at: Utc::now(),
            })
            .unwrap();

        let deleted = store.delete_cases(&["c1".into()]).unwrap();
        assert_eq!(deleted, 1);
        assert!(store.get_case("c1").unwrap().is_none());
        // Cascade removed the verdict (and through it the retrieval trace).
        assert!(store.latest_verdict("c1").unwrap().is_none());
        assert!(store.get_feedback("c1").unwrap().is_none());
        assert!(store.list_attachments_for_case("c1").unwrap().is_empty());
        // c2 must survive.
        assert!(store.get_case("c2").unwrap().is_some());

        // Empty input is a no-op; missing ids return 0 instead of erroring.
        assert_eq!(store.delete_cases(&[]).unwrap(), 0);
        assert_eq!(store.delete_cases(&["ghost".into()]).unwrap(), 0);
    }

    #[test]
    fn migration_adds_attachment_refs_to_existing_db() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cases.sqlite");
        // Simulate a pre-existing DB without the new column.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE cases (id TEXT PRIMARY KEY, created_at TEXT, workspace_id TEXT,
                                     question TEXT, original_text TEXT, masked_text TEXT,
                                     deident_pipeline_id TEXT, status TEXT);
                 CREATE TABLE verdicts (id TEXT PRIMARY KEY, case_id TEXT REFERENCES cases(id),
                                        prompt_version TEXT, provider_id TEXT, model TEXT,
                                        latency_ms INTEGER, input_tokens INTEGER,
                                        output_tokens INTEGER, output_json TEXT,
                                        created_at TEXT);
                 CREATE TABLE retrieval_traces (verdict_id TEXT PRIMARY KEY REFERENCES verdicts(id),
                                                evidence_refs_json TEXT NOT NULL,
                                                past_cases_refs_json TEXT NOT NULL,
                                                online_evidence_refs_json TEXT NOT NULL);",
            )
            .unwrap();
        }
        let _store = CaseStore::open(&path).unwrap();
        // Reopen as a raw connection and confirm the column now exists.
        let conn = Connection::open(&path).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(retrieval_traces)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "attachment_refs_json"));
    }
}
