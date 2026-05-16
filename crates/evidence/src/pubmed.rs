//! PubMed E-utilities adapter (NCBI). Uses `esearch` to resolve PMIDs and
//! `esummary` to fetch metadata. Honours NCBI's mandatory `tool=` and
//! `email=` parameters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::cache::EvidenceCache;
use crate::{EvidenceError, EvidenceItem, EvidenceSource};

const ESEARCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi";
const ESUMMARY_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi";
const TOOL_NAME: &str = "Conclave";

/// PubMed adapter. Requires a contact email per NCBI policy.
pub struct PubMedSource {
    contact_email: String,
    api_key: Option<String>,
    client: reqwest::Client,
    cache: Option<Arc<EvidenceCache>>,
}

impl std::fmt::Debug for PubMedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubMedSource")
            .field("contact_email", &self.contact_email)
            .field("has_api_key", &self.api_key.is_some())
            .field("has_cache", &self.cache.is_some())
            .finish_non_exhaustive()
    }
}

impl PubMedSource {
    /// Build an adapter. `contact_email` is mandatory per NCBI policy.
    pub fn new(contact_email: impl Into<String>) -> Result<Self, EvidenceError> {
        let contact_email = contact_email.into();
        if contact_email.trim().is_empty() {
            return Err(EvidenceError::Config(
                "pubmed contact email cannot be empty".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| EvidenceError::Network(e.to_string()))?;
        Ok(Self {
            contact_email,
            api_key: None,
            client,
            cache: None,
        })
    }

    /// Attach an optional NCBI API key (raises the per-second rate limit).
    #[must_use]
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Attach a cache for query results.
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<EvidenceCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    async fn esearch(&self, query: &str, limit: usize) -> Result<Vec<String>, EvidenceError> {
        let mut req = self.client.get(ESEARCH_URL).query(&[
            ("db", "pubmed"),
            ("retmode", "json"),
            ("retmax", &limit.to_string()),
            ("tool", TOOL_NAME),
            ("email", &self.contact_email),
            ("term", query),
        ]);
        if let Some(key) = &self.api_key {
            req = req.query(&[("api_key", key.as_str())]);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| EvidenceError::Network(e.to_string()))?;
        if resp.status().as_u16() == 429 {
            return Err(EvidenceError::RateLimit);
        }
        if !resp.status().is_success() {
            return Err(EvidenceError::Upstream(format!(
                "esearch {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        #[derive(Deserialize)]
        struct Esearch {
            esearchresult: EsearchResult,
        }
        #[derive(Deserialize)]
        struct EsearchResult {
            #[serde(default)]
            idlist: Vec<String>,
        }
        let parsed: Esearch = resp
            .json()
            .await
            .map_err(|e| EvidenceError::Parse(format!("esearch json: {e}")))?;
        Ok(parsed.esearchresult.idlist)
    }

    async fn esummary(&self, ids: &[String]) -> Result<Vec<EvidenceItem>, EvidenceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let joined = ids.join(",");
        let mut req = self.client.get(ESUMMARY_URL).query(&[
            ("db", "pubmed"),
            ("retmode", "json"),
            ("tool", TOOL_NAME),
            ("email", &self.contact_email),
            ("id", &joined),
        ]);
        if let Some(key) = &self.api_key {
            req = req.query(&[("api_key", key.as_str())]);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| EvidenceError::Network(e.to_string()))?;
        if resp.status().as_u16() == 429 {
            return Err(EvidenceError::RateLimit);
        }
        if !resp.status().is_success() {
            return Err(EvidenceError::Upstream(format!(
                "esummary {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| EvidenceError::Parse(format!("esummary json: {e}")))?;
        let result = body
            .get("result")
            .ok_or_else(|| EvidenceError::Parse("esummary missing `result` object".into()))?;

        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(entry) = result.get(id) else {
                continue;
            };
            let title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let venue = entry
                .get("fulljournalname")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let year = entry
                .get("pubdate")
                .and_then(|v| v.as_str())
                .and_then(parse_year);
            let authors = entry
                .get("authors")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            items.push(EvidenceItem {
                source: "pubmed".into(),
                id: id.clone(),
                title,
                authors,
                year,
                venue,
                abstract_text: None,
                url: format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
            });
        }
        Ok(items)
    }
}

fn parse_year(s: &str) -> Option<u16> {
    s.split(|c: char| !c.is_ascii_digit())
        .find(|p| p.len() == 4)
        .and_then(|p| p.parse().ok())
}

#[async_trait]
impl EvidenceSource for PubMedSource {
    fn id(&self) -> &'static str {
        "pubmed"
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<EvidenceItem>, EvidenceError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(cache) = &self.cache {
            if let Some(hit) = cache.lookup("pubmed", trimmed)? {
                tracing::debug!(query = trimmed, "pubmed cache hit");
                return Ok(hit);
            }
        }
        let ids = self.esearch(trimmed, limit).await?;
        let items = self.esummary(&ids).await?;
        if let Some(cache) = &self.cache {
            let _ = cache.put("pubmed", trimmed, &items);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_email() {
        assert!(PubMedSource::new("").is_err());
        assert!(PubMedSource::new("conclave@example.com").is_ok());
    }

    #[test]
    fn parses_year_from_pubdate() {
        assert_eq!(parse_year("2024 Jun 03"), Some(2024));
        assert_eq!(parse_year("1999 Aug-Sep"), Some(1999));
        assert_eq!(parse_year("Bogus"), None);
    }
}
