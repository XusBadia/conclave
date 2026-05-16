#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::option_if_let_else,
    clippy::missing_const_for_fn,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::significant_drop_tightening,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::or_fun_call,
    clippy::single_char_pattern,
    clippy::struct_field_names,
    clippy::wildcard_imports,
    clippy::map_unwrap_or,
    clippy::needless_match,
    clippy::single_match
)]

//! Online-evidence adapters for the verdict engine.
//!
//! Phase 6 wires PubMed (NCBI E-utilities). Europe PMC and a richer MeSH
//! query generator are out of scope for this slice; the public
//! [`EvidenceSource`] trait makes them straightforward additions later.
//!
//! ## Privacy
//!
//! Adapters never receive the patient case text. The caller is expected
//! to pass a *generated* search query (today derived heuristically from
//! the de-identified case; later from a light-task LLM call). Tests assert
//! that nothing else leaves the device.

mod cache;
mod pubmed;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use cache::EvidenceCache;
pub use pubmed::PubMedSource;

/// Anything that can search a literature corpus and return hits.
#[async_trait]
pub trait EvidenceSource: Send + Sync + std::fmt::Debug {
    /// Stable identifier (`pubmed`, `europepmc`, …).
    fn id(&self) -> &'static str;

    /// Run a search. Implementations should respect their upstream's rate
    /// limits and surface clean errors via [`EvidenceError`].
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<EvidenceItem>, EvidenceError>;
}

/// One bibliographic hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Source id (`pubmed`).
    pub source: String,
    /// Upstream id (PMID, DOI, …).
    pub id: String,
    /// Title of the article.
    pub title: String,
    /// Authors as parsed (best-effort).
    pub authors: Vec<String>,
    /// Year of publication, when present.
    pub year: Option<u16>,
    /// Journal / venue name.
    pub venue: Option<String>,
    /// Abstract text, when present.
    pub abstract_text: Option<String>,
    /// Canonical URL for the entry.
    pub url: String,
}

/// Failure modes for evidence adapters.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EvidenceError {
    #[error("network error: {0}")]
    Network(String),
    #[error("rate limited")]
    RateLimit,
    #[error("upstream returned an error: {0}")]
    Upstream(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("cache error: {0}")]
    Cache(String),
}
