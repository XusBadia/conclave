//! Simple SQLite-backed query/result cache with a configurable TTL.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::{EvidenceError, EvidenceItem};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS evidence_queries (
    hash TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    query_text TEXT NOT NULL,
    ran_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
";

const DEFAULT_TTL_DAYS: i64 = 30;

/// On-disk cache for evidence queries.
#[derive(Debug)]
pub struct EvidenceCache {
    conn: Mutex<Connection>,
    path: PathBuf,
    ttl_days: i64,
}

impl EvidenceCache {
    /// Open or create the cache at `path`. Uses the default 30-day TTL.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EvidenceError> {
        Self::open_with_ttl(path, DEFAULT_TTL_DAYS)
    }

    /// Open with a custom TTL (in days). Pass 0 to disable expiry.
    pub fn open_with_ttl(path: impl AsRef<Path>, ttl_days: i64) -> Result<Self, EvidenceError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| EvidenceError::Cache(format!("mkdir {}: {e}", parent.display())))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| EvidenceError::Cache(format!("open {}: {e}", path.display())))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| EvidenceError::Cache(format!("schema: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
            ttl_days,
        })
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, EvidenceError> {
        self.conn
            .lock()
            .map_err(|_| EvidenceError::Cache("connection mutex poisoned".into()))
    }

    /// Path the connection was opened against.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Lookup a cached result for `(source, query)`. Returns `None` when
    /// missing or expired.
    pub fn lookup(
        &self,
        source: &str,
        query: &str,
    ) -> Result<Option<Vec<EvidenceItem>>, EvidenceError> {
        let hash = query_hash(source, query);
        let conn = self.lock_conn()?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT ran_at, payload_json FROM evidence_queries WHERE hash = ?1",
                params![hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| EvidenceError::Cache(e.to_string()))?;
        let Some((ran_at_raw, payload)) = row else {
            return Ok(None);
        };
        if self.ttl_days > 0 {
            let ran_at = DateTime::parse_from_rfc3339(&ran_at_raw)
                .map_err(|e| EvidenceError::Cache(format!("bad ran_at: {e}")))?
                .with_timezone(&Utc);
            if Utc::now() - ran_at > Duration::days(self.ttl_days) {
                return Ok(None);
            }
        }
        let parsed: Vec<EvidenceItem> = serde_json::from_str(&payload)
            .map_err(|e| EvidenceError::Cache(format!("parse payload: {e}")))?;
        Ok(Some(parsed))
    }

    /// Persist a result for `(source, query)`.
    pub fn put(
        &self,
        source: &str,
        query: &str,
        items: &[EvidenceItem],
    ) -> Result<(), EvidenceError> {
        let hash = query_hash(source, query);
        let payload = serde_json::to_string(items)
            .map_err(|e| EvidenceError::Cache(format!("serialise: {e}")))?;
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO evidence_queries (hash, source, query_text, ran_at, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(hash) DO UPDATE SET
                   ran_at = excluded.ran_at,
                   payload_json = excluded.payload_json",
            params![hash, source, query, Utc::now().to_rfc3339(), payload],
        )
        .map_err(|e| EvidenceError::Cache(e.to_string()))?;
        Ok(())
    }

    /// Wipe the cache.
    pub fn clear(&self) -> Result<(), EvidenceError> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM evidence_queries", [])
            .map_err(|e| EvidenceError::Cache(e.to_string()))?;
        Ok(())
    }
}

fn query_hash(source: &str, query: &str) -> String {
    let mut h = Sha256::new();
    h.update(source.as_bytes());
    h.update(b"\0");
    h.update(query.as_bytes());
    let bytes = h.finalize();
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> EvidenceItem {
        EvidenceItem {
            source: "pubmed".into(),
            id: "12345".into(),
            title: "Sample".into(),
            authors: vec!["A".into()],
            year: Some(2024),
            venue: Some("Lancet".into()),
            abstract_text: Some("abstract".into()),
            url: "https://pubmed.ncbi.nlm.nih.gov/12345/".into(),
        }
    }

    #[test]
    fn round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = EvidenceCache::open(tmp.path().join("evidence.sqlite")).unwrap();
        assert!(cache.lookup("pubmed", "test").unwrap().is_none());
        cache.put("pubmed", "test", &[sample_item()]).unwrap();
        let hit = cache.lookup("pubmed", "test").unwrap().unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].id, "12345");
    }

    #[test]
    fn ttl_zero_never_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = EvidenceCache::open_with_ttl(tmp.path().join("evidence.sqlite"), 0).unwrap();
        cache.put("pubmed", "test", &[sample_item()]).unwrap();
        assert!(cache.lookup("pubmed", "test").unwrap().is_some());
    }

    #[test]
    fn clear_empties_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = EvidenceCache::open(tmp.path().join("evidence.sqlite")).unwrap();
        cache.put("pubmed", "test", &[sample_item()]).unwrap();
        cache.clear().unwrap();
        assert!(cache.lookup("pubmed", "test").unwrap().is_none());
    }
}
