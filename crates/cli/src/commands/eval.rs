//! `conclave-cli eval` — run a batch of cases through the verdict pipeline and
//! score 3-level concordance against a known committee decision, stratified by
//! certainty (the calibration signal).
//!
//! Privacy: the manifest and any `text_file` cases are read from a LOCAL path
//! the operator supplies — nothing here is committed. Keep a real-patient
//! corpus outside the repo (the git-ignored `eval-corpus/` is the suggested
//! home). Each case is de-identified and persisted exactly like `case new`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Args;
use serde::Deserialize;

use conclave_deident::PipelineDeidentifier;
use conclave_verdict::{
    CaseOutcome, ConcordanceReport, DataBoundaryMode, DecisionCategory, Tally, VerdictOptions,
    VerdictPipeline,
};

use super::case::{build_provider, case_store};
use super::CommandContext;

/// Arguments for the `eval` subcommand.
#[derive(Debug, Args)]
pub(crate) struct EvalArgs {
    /// Path to the JSON manifest: an array of cases with expected decisions.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,
    /// Provider id. Overrides `config.providers.default`.
    #[arg(long)]
    provider: Option<String>,
    /// Explicit model id (provider default otherwise).
    #[arg(long)]
    model: Option<String>,
    /// Data boundary: local_only, deid_cloud (default), explicit_phi.
    #[arg(long, default_value = "deid_cloud")]
    data_boundary: String,
    /// KB relevance floor (cosine; 0 = disabled). Tune per corpus.
    #[arg(long, default_value_t = 0.0)]
    kb_min_relevance: f32,
    /// Inject up to N past cases. Default 0 keeps cases independent — the
    /// right choice for a clean concordance study (no cross-case leakage).
    #[arg(long, default_value_t = 0)]
    past_cases_k: usize,
    /// Write the full report (summary + per-case rows) as JSON to this path.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
}

/// One row of the eval manifest.
#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    text_file: Option<PathBuf>,
    #[serde(default = "default_question")]
    question: String,
    expected_category: DecisionCategory,
}

fn default_question() -> String {
    "¿Cuál es el manejo recomendado?".to_owned()
}

pub(crate) async fn run(ctx: &CommandContext, args: EvalArgs) -> Result<()> {
    let manifest_dir = args
        .manifest
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let raw = std::fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading manifest {}", args.manifest.display()))?;
    let cases: Vec<EvalCase> = serde_json::from_str(&raw)
        .with_context(|| format!("parsing manifest {}", args.manifest.display()))?;
    if cases.is_empty() {
        anyhow::bail!("manifest {} has no cases", args.manifest.display());
    }

    let workspace = ctx.resolve_workspace(None)?;
    let boundary = DataBoundaryMode::from_db_str(&args.data_boundary);
    let provider_id = args
        .provider
        .clone()
        .or_else(|| ctx.config.providers.default.clone())
        .ok_or_else(|| anyhow!("no provider configured — pass --provider <id>"))?;
    let provider = build_provider(&provider_id, args.model.clone()).await?;
    if matches!(boundary, DataBoundaryMode::LocalOnly) && provider.requires_network() {
        anyhow::bail!("local_only blocks network provider `{provider_id}`");
    }

    let store = case_store(ctx, &workspace)?;
    let embedder = ctx.default_embedder();
    let repo = ctx.open_repository(&workspace, &embedder).await?;
    let pipeline = VerdictPipeline::new(
        workspace.clone(),
        Box::new(PipelineDeidentifier::new()),
        embedder,
        repo,
        provider,
        store,
    );

    let mut options = VerdictOptions::default();
    options.top_k = ctx.config.rag.top_k;
    options.kb_min_relevance = args.kb_min_relevance;
    options.past_cases_k = args.past_cases_k;
    options.data_boundary_mode = boundary;
    if let Some(lang) = workspace.language.clone() {
        options.output_language = lang;
    }

    let mut outcomes = Vec::with_capacity(cases.len());
    for case in &cases {
        let text = resolve_text(case, &manifest_dir)?;
        eprintln!("running {} …", case.id);
        let run = pipeline
            .run(&text, &case.question, &[], &options)
            .await
            .with_context(|| format!("case `{}` failed", case.id))?;
        outcomes.push(CaseOutcome::score(
            case.id.clone(),
            case.expected_category,
            &run.verdict,
        ));
    }

    let report = ConcordanceReport::from_outcomes(outcomes);
    print_report(&report);

    if let Some(path) = &args.output {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("\nwrote report to {}", path.display());
    }
    Ok(())
}

fn resolve_text(case: &EvalCase, manifest_dir: &Path) -> Result<String> {
    match (&case.text, &case.text_file) {
        (Some(t), _) => Ok(t.clone()),
        (None, Some(file)) => {
            let path = if file.is_absolute() {
                file.clone()
            } else {
                manifest_dir.join(file)
            };
            std::fs::read_to_string(&path)
                .with_context(|| format!("reading case `{}` from {}", case.id, path.display()))
        }
        (None, None) => Err(anyhow!(
            "case `{}` has neither `text` nor `text_file`",
            case.id
        )),
    }
}

fn print_report(report: &ConcordanceReport) {
    println!("\nCONCORDANCE ({} cases)", report.overall.total());
    println!("─────────────────────────");
    println!("overall: {}", fmt_tally(&report.overall));
    println!("by certainty (calibration — expect high ≥ medium ≥ low):");
    println!("  high   : {}", fmt_tally(&report.by_certainty.high));
    println!("  medium : {}", fmt_tally(&report.by_certainty.medium));
    println!("  low    : {}", fmt_tally(&report.by_certainty.low));

    println!("\nper-case");
    println!("────────");
    for o in &report.outcomes {
        println!(
            "  {:<16}  exp={:<18} pred={:<18} {:<11} {:<7} {:<12} {}",
            truncate(&o.id, 16),
            format!("{:?}", o.expected),
            format!("{:?}", o.predicted),
            format!("{:?}", o.concordance),
            o.certainty.label(),
            o.data_completeness.label(),
            truncate(&o.action, 60),
        );
    }
}

fn fmt_tally(t: &Tally) -> String {
    let rate = t
        .strict_rate()
        .map_or_else(|| "—".to_owned(), |r| format!("{:.0}%", r * 100.0));
    format!(
        "concordant {} ({})  partial {}  discordant {}  (n={})",
        t.concordant,
        rate,
        t.partial,
        t.discordant,
        t.total(),
    )
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
    }
    out
}
