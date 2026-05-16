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
    status TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS cases_created_at_idx ON cases(created_at DESC);

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
    const fn as_db_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Modify => "modify",
            Self::Reject => "reject",
        }
    }
    fn from_db_str(s: &str) -> Option<Self> {
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
        Ok(Self { conn, path })
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
                    deident_pipeline_id, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    c.id,
                    c.created_at.to_rfc3339(),
                    c.workspace_id,
                    c.question,
                    c.original_text,
                    c.masked_text,
                    c.deident_pipeline_id,
                    c.status.as_db_str(),
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
                   (verdict_id, evidence_refs_json, past_cases_refs_json, online_evidence_refs_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    t.verdict_id,
                    serde_json::to_string(&t.evidence_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&t.past_cases_refs).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&t.online_evidence_refs).unwrap_or_else(|_| "[]".into()),
                ],
            )
            .map_err(map_sql)?;
        Ok(())
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

    /// List cases, most-recent first.
    pub fn list_cases(&self, limit: usize) -> Result<Vec<CaseRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, workspace_id, question, original_text, masked_text,
                        deident_pipeline_id, status
                 FROM cases
                 ORDER BY created_at DESC
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
                        deident_pipeline_id, status
                 FROM cases WHERE id = ?1",
                params![id],
                row_to_case,
            )
            .optional()
            .map_err(map_sql)
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
}

fn row_to_case(r: &rusqlite::Row<'_>) -> rusqlite::Result<CaseRecord> {
    Ok(CaseRecord {
        id: r.get(0)?,
        created_at: parse_dt(&r.get::<_, String>(1)?),
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
        CaseRecord {
            id: id.into(),
            created_at: Utc::now(),
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
}
