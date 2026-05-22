//! Internal clinical-service abstraction layer.
//!
//! The first implementation remains fully local. The layer exists so future
//! terminology, extraction, and evidence services can be swapped without
//! coupling the verdict pipeline to one backend or a generic LLM call.

use std::collections::BTreeMap;
use std::path::Path;

use async_trait::async_trait;

use conclave_core::{Error, Result};
use conclave_deident::{DeidentResult, Deidentifier, PipelineDeidentifier};

/// De-identification service boundary.
pub trait DeidentService: Send + Sync {
    fn id(&self) -> &'static str;
    fn deidentify(&self, text: &str) -> Result<DeidentResult>;
}

/// Local regex + heuristic implementation backed by `conclave-deident`.
#[derive(Debug, Clone, Default)]
pub struct LocalDeidentService {
    inner: PipelineDeidentifier,
}

impl LocalDeidentService {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeidentService for LocalDeidentService {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    fn deidentify(&self, text: &str) -> Result<DeidentResult> {
        self.inner.deidentify(text)
    }
}

/// Terminology search / validation boundary.
#[async_trait]
pub trait TerminologyService: Send + Sync {
    fn id(&self) -> &'static str;
    async fn search(&self, system: &str, query: &str) -> Result<Vec<TerminologyHit>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminologyHit {
    pub system: String,
    pub code: String,
    pub label: String,
}

/// Local CSV-backed terminology catalogs. Each `*.csv` file is a system
/// (`cie10.csv`, `loinc.csv`, ...). Headers may be `code,label` or the first
/// two columns are used.
#[derive(Debug, Clone, Default)]
pub struct CsvTerminologyService {
    catalogs: BTreeMap<String, Vec<TerminologyHit>>,
}

impl CsvTerminologyService {
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(Self::default());
        }
        let mut catalogs: BTreeMap<String, Vec<TerminologyHit>> = BTreeMap::new();
        for entry in std::fs::read_dir(dir).map_err(|e| Error::io_at(dir, e))? {
            let entry = entry.map_err(|e| Error::invalid_config(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("csv") {
                continue;
            }
            let system = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            let hits = load_catalog_csv(&system, &path)?;
            catalogs.insert(system, hits);
        }
        Ok(Self { catalogs })
    }

    pub fn is_empty(&self) -> bool {
        self.catalogs.values().all(Vec::is_empty)
    }
}

#[async_trait]
impl TerminologyService for CsvTerminologyService {
    fn id(&self) -> &'static str {
        "csv-terminology-v1"
    }

    async fn search(&self, system: &str, query: &str) -> Result<Vec<TerminologyHit>> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let systems: Vec<&str> = if system.trim().is_empty() || system == "*" {
            self.catalogs.keys().map(String::as_str).collect()
        } else {
            vec![system]
        };
        let mut out = Vec::new();
        for system in systems {
            if let Some(rows) = self.catalogs.get(&system.to_ascii_lowercase()) {
                for hit in rows {
                    if hit.code.to_ascii_lowercase().contains(&query)
                        || hit.label.to_ascii_lowercase().contains(&query)
                    {
                        out.push(hit.clone());
                    }
                    if out.len() >= 20 {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}

fn load_catalog_csv(system: &str, path: &Path) -> Result<Vec<TerminologyHit>> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| {
        Error::invalid_config(format!("reading terminology {}: {e}", path.display()))
    })?;
    let headers = reader
        .headers()
        .map_err(|e| Error::invalid_config(format!("reading terminology headers: {e}")))?
        .clone();
    let code_idx = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("code"))
        .unwrap_or(0);
    let label_idx = headers
        .iter()
        .position(|h| {
            h.eq_ignore_ascii_case("label")
                || h.eq_ignore_ascii_case("display")
                || h.eq_ignore_ascii_case("term")
                || h.eq_ignore_ascii_case("description")
        })
        .unwrap_or(1);
    let mut out = Vec::new();
    for row in reader.records() {
        let row =
            row.map_err(|e| Error::invalid_config(format!("reading terminology row: {e}")))?;
        let code = row.get(code_idx).unwrap_or_default().trim();
        let label = row.get(label_idx).unwrap_or_default().trim();
        if code.is_empty() || label.is_empty() {
            continue;
        }
        out.push(TerminologyHit {
            system: system.to_owned(),
            code: code.to_owned(),
            label: label.to_owned(),
        });
    }
    Ok(out)
}

/// Evidence service boundary for online literature or local evidence planes.
#[async_trait]
pub trait EvidenceService: Send + Sync {
    fn id(&self) -> &'static str;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ExternalEvidenceHit>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEvidenceHit {
    pub source: String,
    pub id: String,
    pub title: String,
    pub url: String,
    pub abstract_text: Option<String>,
}

/// Structured clinical extraction boundary.
#[async_trait]
pub trait ExtractionService: Send + Sync {
    fn id(&self) -> &'static str;
    async fn extract(&self, text: &str, entity_types: &[String]) -> Result<Vec<ExtractionHit>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionHit {
    pub entity_type: String,
    pub text: String,
    pub normalized: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn csv_terminology_searches_by_code_and_label() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("cie10.csv"),
            "code,label\nC20,Malignant neoplasm of rectum\nI21,Acute myocardial infarction\n",
        )
        .unwrap();
        let svc = CsvTerminologyService::from_dir(tmp.path()).unwrap();
        let hits = svc.search("cie10", "rectum").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].code, "C20");
        let hits = svc.search("*", "I21").await.unwrap();
        assert_eq!(hits[0].label, "Acute myocardial infarction");
    }
}
