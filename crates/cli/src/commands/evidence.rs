//! `conclave-cli evidence` — query PubMed for recent literature.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};

use conclave_evidence::{EuropePmcSource, EvidenceCache, EvidenceSource, PubMedSource};

use super::CommandContext;

/// Arguments for the `evidence` subcommand.
#[derive(Debug, Args)]
pub(crate) struct EvidenceArgs {
    #[command(subcommand)]
    action: EvidenceAction,
}

#[derive(Debug, Subcommand)]
enum EvidenceAction {
    /// Run a query against PubMed.
    Search {
        /// Query string (supports PubMed boolean operators and [Mesh] tags).
        query: String,
        /// Max results to return.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Contact email required by NCBI policy
        /// (defaults to $CONCLAVE_NCBI_EMAIL).
        #[arg(long)]
        email: Option<String>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Source: pubmed, europepmc, or auto (PubMed then Europe PMC).
        #[arg(long, default_value = "auto")]
        source: String,
    },
    /// Wipe the on-disk evidence cache.
    CacheClear,
}

pub(crate) async fn run(ctx: &CommandContext, args: EvidenceArgs) -> Result<()> {
    let cache_path = ctx.paths.cache_dir().join("evidence.sqlite");
    match args.action {
        EvidenceAction::Search {
            query,
            limit,
            email,
            json,
            source,
        } => {
            let cache =
                Arc::new(EvidenceCache::open(&cache_path).with_context(|| {
                    format!("opening evidence cache at {}", cache_path.display())
                })?);
            print_banner();
            let items = match source.as_str() {
                "pubmed" => {
                    let email = email
                        .or_else(|| std::env::var("CONCLAVE_NCBI_EMAIL").ok())
                        .ok_or_else(|| {
                            anyhow!(
                                "PubMed requires a contact email — pass --email or set \
                                 $CONCLAVE_NCBI_EMAIL"
                            )
                        })?;
                    PubMedSource::new(&email)?
                        .with_cache(Arc::clone(&cache))
                        .search(&query, limit)
                        .await?
                }
                "europepmc" => {
                    EuropePmcSource::new()?
                        .with_cache(Arc::clone(&cache))
                        .search(&query, limit)
                        .await?
                }
                "auto" => search_auto(&query, limit, email.as_deref(), Arc::clone(&cache)).await?,
                other => anyhow::bail!("unknown source `{other}`; use pubmed, europepmc, or auto"),
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&items).unwrap_or_default()
                );
                return Ok(());
            }
            if items.is_empty() {
                println!("(no results)");
                return Ok(());
            }
            for (i, it) in items.iter().enumerate() {
                let authors = if it.authors.is_empty() {
                    String::new()
                } else if it.authors.len() <= 3 {
                    it.authors.join(", ")
                } else {
                    format!("{} et al.", it.authors[0])
                };
                println!(
                    "{:>2}. {}{} ({})",
                    i + 1,
                    it.title,
                    it.year.map_or(String::new(), |y| format!(" — {y}")),
                    it.venue.as_deref().unwrap_or("?"),
                );
                if !authors.is_empty() {
                    println!("    {authors}");
                }
                println!("    PMID:{} · {}", it.id, it.url);
            }
        }
        EvidenceAction::CacheClear => {
            let cache = EvidenceCache::open(&cache_path)
                .with_context(|| format!("opening evidence cache at {}", cache_path.display()))?;
            cache.clear()?;
            println!("evidence cache cleared ({})", cache_path.display());
        }
    }
    Ok(())
}

async fn search_auto(
    query: &str,
    limit: usize,
    email: Option<&str>,
    cache: Arc<EvidenceCache>,
) -> Result<Vec<conclave_evidence::EvidenceItem>> {
    let mut first_error: Option<String> = None;
    let email = email
        .map(str::to_owned)
        .or_else(|| std::env::var("CONCLAVE_NCBI_EMAIL").ok());
    if let Some(email) = email {
        match PubMedSource::new(email).map(|s| s.with_cache(Arc::clone(&cache))) {
            Ok(pubmed) => match pubmed.search(query, limit).await {
                Ok(items) if !items.is_empty() => return Ok(items),
                Ok(_) => {}
                Err(e) => first_error = Some(e.to_string()),
            },
            Err(e) => first_error = Some(e.to_string()),
        }
    }
    EuropePmcSource::new()?
        .with_cache(cache)
        .search(query, limit)
        .await
        .map_err(|e| anyhow!(first_error.unwrap_or_else(|| e.to_string())))
}

fn print_banner() {
    eprintln!("⚠  Connecting to external evidence sources (PubMed / Europe PMC).");
    eprintln!("   No case text is sent — only the query string above.\n");
}
