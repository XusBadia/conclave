//! SQLite-backed persistence for cases, verdicts, retrieval traces and
//! feedback. Lives in the same `metadata.sqlite` as document metadata so
//! a single workspace database holds everything.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

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
    case_date TEXT NOT NULL DEFAULT ''
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
";

/// Outcome flag for a case lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    /// Verdict produced and persisted.
    Completed,
    /// LLM call or validation failed.
    Failed,
}

impl CaseStatus {
    const fn as_db_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "completed" => Some(Self::Completed),
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
                    deident_pipeline_id, status, case_date)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
                ],
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

    /// List cases ordered by `case_date` (user-facing date) descending.
    /// Falls back to `created_at` on ties for stable order.
    pub fn list_cases(&self, limit: usize) -> Result<Vec<CaseRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, workspace_id, question, original_text, masked_text,
                        deident_pipeline_id, status, case_date
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
                        deident_pipeline_id, status, case_date
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
                "SELECT COUNT(*) FROM cases WHERE status = 'completed'",
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
            status: CaseStatus::Completed,
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
