//! Europe PMC REST adapter. Used as a fallback when PubMed returns no
//! usable results or is unavailable. Europe PMC does not require a contact
//! email for the public search endpoint.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::cache::EvidenceCache;
use crate::{EvidenceError, EvidenceItem, EvidenceSource};

const SEARCH_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";

#[derive(Clone)]
pub struct EuropePmcSource {
    client: reqwest::Client,
    cache: Option<Arc<EvidenceCache>>,
}

impl std::fmt::Debug for EuropePmcSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EuropePmcSource")
            .field("has_cache", &self.cache.is_some())
            .finish_non_exhaustive()
    }
}

impl EuropePmcSource {
    pub fn new() -> Result<Self, EvidenceError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| EvidenceError::Network(e.to_string()))?;
        Ok(Self {
            client,
            cache: None,
        })
    }

    #[must_use]
    pub fn with_cache(mut self, cache: Arc<EvidenceCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    async fn search_uncached(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<EvidenceItem>, EvidenceError> {
        let resp = self
            .client
            .get(SEARCH_URL)
            .query(&[
                ("query", query),
                ("format", "json"),
                ("pageSize", &limit.to_string()),
            ])
            .send()
            .await
            .map_err(|e| EvidenceError::Network(e.to_string()))?;
        if resp.status().as_u16() == 429 {
            return Err(EvidenceError::RateLimit);
        }
        if !resp.status().is_success() {
            return Err(EvidenceError::Upstream(format!(
                "europepmc search {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        let parsed: EuropePmcResponse = resp
            .json()
            .await
            .map_err(|e| EvidenceError::Parse(format!("europepmc json: {e}")))?;
        Ok(parsed
            .result_list
            .result
            .into_iter()
            .map(EuropePmcRecord::into_item)
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct EuropePmcResponse {
    #[serde(rename = "resultList")]
    result_list: EuropePmcResultList,
}

#[derive(Debug, Deserialize)]
struct EuropePmcResultList {
    #[serde(default)]
    result: Vec<EuropePmcRecord>,
}

#[derive(Debug, Deserialize)]
struct EuropePmcRecord {
    #[serde(default)]
    id: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "authorString")]
    author_string: Option<String>,
    #[serde(rename = "pubYear")]
    pub_year: Option<String>,
    #[serde(rename = "journalTitle")]
    journal_title: Option<String>,
    #[serde(rename = "abstractText")]
    abstract_text: Option<String>,
    pmid: Option<String>,
    doi: Option<String>,
}

impl EuropePmcRecord {
    fn into_item(self) -> EvidenceItem {
        let id = self.pmid.clone().unwrap_or(self.id);
        let source = if self.source.is_empty() {
            "europepmc".to_owned()
        } else {
            format!("europepmc:{}", self.source.to_ascii_lowercase())
        };
        let url = if let Some(pmid) = &self.pmid {
            format!("https://europepmc.org/article/MED/{pmid}")
        } else if let Some(doi) = &self.doi {
            format!("https://europepmc.org/search?query=DOI:{doi}")
        } else {
            format!("https://europepmc.org/article/{}/{}", self.source, id)
        };
        EvidenceItem {
            source,
            id,
            title: self.title,
            authors: self
                .author_string
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            year: self.pub_year.and_then(|y| y.parse().ok()),
            venue: self.journal_title,
            abstract_text: self.abstract_text,
            url,
        }
    }
}

#[async_trait]
impl EvidenceSource for EuropePmcSource {
    fn id(&self) -> &'static str {
        "europepmc"
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<EvidenceItem>, EvidenceError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(cache) = &self.cache {
            if let Some(hit) = cache.lookup("europepmc", trimmed)? {
                tracing::debug!(query = trimmed, "europepmc cache hit");
                return Ok(hit);
            }
        }
        let items = self.search_uncached(trimmed, limit).await?;
        if let Some(cache) = &self.cache {
            let _ = cache.put("europepmc", trimmed, &items);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_record_to_evidence_item() {
        let item = EuropePmcRecord {
            id: "123".into(),
            source: "MED".into(),
            title: "Title".into(),
            author_string: Some("A One, B Two".into()),
            pub_year: Some("2024".into()),
            journal_title: Some("Journal".into()),
            abstract_text: Some("Abstract".into()),
            pmid: Some("123".into()),
            doi: None,
        }
        .into_item();
        assert_eq!(item.source, "europepmc:med");
        assert_eq!(item.id, "123");
        assert_eq!(item.authors, vec!["A One", "B Two"]);
        assert_eq!(item.year, Some(2024));
        assert!(item.url.contains("europepmc.org"));
    }
}
