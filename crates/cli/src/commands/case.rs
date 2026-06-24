//! `conclave-cli case` — submit a clinical case to the verdict engine and
//! browse stored cases.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};

use conclave_deident::PipelineDeidentifier;
use conclave_evidence::{
    EuropePmcSource, EvidenceCache, EvidenceItem, EvidenceSource, PubMedSource,
};
use conclave_providers::{
    secrets, AnthropicOAuthProvider, AnthropicProvider, LlmProvider, OllamaProvider,
    OpenAIOAuthProvider, OpenAiProvider, OpenRouterProvider,
};
use conclave_verdict::{
    load_skill, run_workflow, CaseStore, CertaintyLevel, ClinicalWorkflow, CsvTerminologyService,
    DataBoundaryMode, FeedbackKind, ReviewDecision, ReviewMetadataRecord, Verdict, VerdictOptions,
    VerdictPipeline,
};

use super::CommandContext;

/// Arguments for the `case` subcommand.
#[derive(Debug, Args)]
pub(crate) struct CaseArgs {
    #[command(subcommand)]
    action: CaseAction,
}

#[derive(Debug, Subcommand)]
enum CaseAction {
    /// Run the verdict pipeline over an inline case, file or stdin.
    New {
        /// Read the case from `--file` instead of `text`/stdin.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// Inline case text.
        text: Option<String>,
        /// Question for the committee.
        #[arg(long, default_value = "¿Cuál es el manejo recomendado?")]
        question: String,
        /// Provider id (`anthropic`, `openai`, `openrouter`, `ollama`).
        /// Overrides `config.providers.default`.
        #[arg(long)]
        provider: Option<String>,
        /// Explicit model id (provider default otherwise).
        #[arg(long)]
        model: Option<String>,
        /// Skip the de-identified preview / confirm step.
        #[arg(long)]
        yes: bool,
        /// Print the verdict as JSON instead of pretty.
        #[arg(long)]
        json: bool,
        /// Data boundary: local_only, deid_cloud (default), explicit_phi.
        #[arg(long, default_value = "deid_cloud")]
        data_boundary: String,
        /// Retain raw narrative locally after a successful run.
        #[arg(long)]
        retain_raw_text: bool,
        /// Active skill id to apply as a prompt overlay.
        #[arg(long)]
        skill: Option<String>,
        /// Opt in to external literature lookup (PubMed, then Europe PMC).
        #[arg(long)]
        online_evidence: bool,
        /// Override the generated de-identified literature query.
        #[arg(long)]
        external_evidence_query: Option<String>,
        /// Contact email for PubMed. Defaults to $CONCLAVE_NCBI_EMAIL;
        /// Europe PMC fallback does not require this.
        #[arg(long)]
        ncbi_email: Option<String>,
    },
    /// List the most recent cases.
    List {
        /// How many to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a single case + its latest verdict.
    Show {
        /// Case id (as printed by `list`).
        id: String,
        /// Print raw JSON instead of pretty output.
        #[arg(long)]
        json: bool,
    },
    /// Purge locally retained raw PHI for one case.
    PurgePhi {
        /// Case id.
        id: String,
    },
    /// Purge raw attachment files while keeping de-identified extracted text.
    PurgeAttachments {
        /// Case id.
        id: String,
    },
    /// Finalize a review-ready verdict with clinician metadata.
    Finalize {
        /// Case id.
        id: String,
        /// Decision: accept, modify, reject.
        #[arg(long, default_value = "accept")]
        decision: String,
        /// Optional reviewer name.
        #[arg(long)]
        reviewer: Option<String>,
        /// Optional reviewer role.
        #[arg(long)]
        role: Option<String>,
        /// Optional note.
        #[arg(long)]
        note: Option<String>,
        /// Optional JSON file containing the clinician-edited final verdict.
        #[arg(long, value_name = "PATH")]
        final_json: Option<PathBuf>,
    },
    /// Run a deterministic local workflow over a stored verdict.
    Workflow {
        /// Case id.
        id: String,
        /// chart_summary, med_rec_discrepancy, guideline_review,
        /// discharge_handoff, coding_audit, structured_extraction_fhir_diff.
        #[arg(long)]
        workflow: String,
        /// Optional terminology catalog directory for coding_audit.
        #[arg(long, value_name = "DIR")]
        terminology_dir: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) async fn run(ctx: &CommandContext, args: CaseArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let store = case_store(ctx, &workspace)?;
    match args.action {
        CaseAction::New {
            file,
            text,
            question,
            provider,
            model,
            yes,
            json,
            data_boundary,
            retain_raw_text,
            skill,
            online_evidence,
            external_evidence_query,
            ncbi_email,
        } => {
            let case_text = read_input(file.as_deref(), text.as_deref())?;

            // Preview de-identified text first unless --yes.
            let preview = PipelineDeidentifier::new().run(&case_text)?;
            if !yes {
                eprintln!("--- de-identified preview ---");
                eprintln!("{}", preview.masked_text);
                eprintln!("--- end preview ---\n");
                eprintln!("Spans: {}", preview.spans.len());
                eprintln!("Strict clean: {}", preview.strict_mode_clean);
                eprintln!("(re-run with --yes to skip this preview)\n");
            }

            let provider_id = provider
                .or_else(|| ctx.config.providers.default.clone())
                .ok_or_else(|| {
                    anyhow!(
                        "no provider configured — run `providers set anthropic` first \
                         or pass --provider <id>"
                    )
                })?;
            let provider_arc = build_provider(&provider_id, model.clone()).await?;
            let boundary = DataBoundaryMode::from_db_str(&data_boundary);
            if matches!(boundary, DataBoundaryMode::LocalOnly) && provider_arc.requires_network() {
                anyhow::bail!("local_only blocks network provider `{provider_id}`");
            }
            if matches!(boundary, DataBoundaryMode::LocalOnly) && online_evidence {
                anyhow::bail!("local_only blocks online evidence lookup");
            }
            let active_skill = if let Some(skill_id) = &skill {
                let user_skills = ctx.paths.config_dir().join("skills");
                let workspace_skills = ctx.paths.workspace_dir(&workspace.id).join("skills");
                let s = load_skill(skill_id, Some(&user_skills), Some(&workspace_skills))?
                    .ok_or_else(|| anyhow!("skill `{skill_id}` not found"))?;
                if !s.allows_mode(boundary) {
                    anyhow::bail!(
                        "skill `{}` does not allow data boundary `{}`",
                        s.id,
                        boundary.as_db_str()
                    );
                }
                Some(s)
            } else {
                None
            };

            let embedder = ctx.default_embedder();
            let repo = ctx.open_repository(&workspace, &embedder).await?;
            let pipeline = VerdictPipeline::new(
                workspace.clone(),
                Box::new(PipelineDeidentifier::new()),
                embedder,
                repo,
                provider_arc,
                store,
            );
            let mut options = VerdictOptions::default();
            options.top_k = ctx.config.rag.top_k;
            if let Some(lang) = workspace.language.clone() {
                options.output_language = lang;
            }
            options.data_boundary_mode = boundary;
            options.retain_raw_text = retain_raw_text;
            if online_evidence {
                options.external_evidence = fetch_external_evidence(
                    ctx,
                    external_evidence_query.as_deref(),
                    &preview.masked_text,
                    &question,
                    ncbi_email.as_deref(),
                )
                .await?;
            }
            if let Some(s) = active_skill {
                options.active_skill_id = Some(s.id);
                options.active_skill_instructions = Some(s.body);
            }
            let run = pipeline
                .run(&case_text, &question, &[], &options)
                .await
                .context("verdict pipeline failed")?;

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&run.verdict).unwrap_or_default()
                );
            } else {
                render_pretty(&run.case.id, &run.verdict);
            }
            eprintln!(
                "\nverdict_id={} provider={} model={} latency={}ms tokens={}+{}",
                run.verdict_record.id,
                run.verdict_record.provider_id,
                run.verdict_record.model,
                run.verdict_record.latency_ms,
                run.verdict_record.input_tokens,
                run.verdict_record.output_tokens,
            );
        }
        CaseAction::List { limit } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            let cases = store.list_cases(limit)?;
            if cases.is_empty() {
                println!("(no cases in workspace `{}`)", workspace.id);
            } else {
                for c in cases {
                    let summary: String = c.question.chars().take(70).collect();
                    println!(
                        "{:<36}  {:<20}  {:<10}  {}",
                        c.id,
                        c.created_at.format("%Y-%m-%d %H:%M:%S"),
                        format!("{:?}", c.status).to_lowercase(),
                        summary,
                    );
                }
            }
        }
        CaseAction::Show { id, json } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            let case = store
                .get_case(&id)?
                .ok_or_else(|| anyhow!("case `{id}` not found"))?;
            let verdict_row = store
                .latest_verdict(&id)?
                .ok_or_else(|| anyhow!("no verdict found for case `{id}`"))?;
            let verdict: Verdict = serde_json::from_str(&verdict_row.output_json)
                .with_context(|| format!("stored verdict for `{id}` does not parse as JSON"))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&verdict).unwrap_or_default()
                );
            } else {
                println!("id:           {}", case.id);
                println!("created:      {}", case.created_at.to_rfc3339());
                println!("question:     {}", case.question);
                println!("provider:     {}", verdict_row.provider_id);
                println!("model:        {}", verdict_row.model);
                println!("latency:      {}ms", verdict_row.latency_ms);
                if let Some(audit) = store.latest_audit_for_case(&id)? {
                    println!(
                        "audit:        {} · {}",
                        audit.id,
                        audit.data_boundary_mode.as_db_str()
                    );
                    println!("prompt hash:  {}", audit.prompt_sha256);
                    println!("output hash:  {}", audit.output_sha256);
                }
                println!();
                render_pretty(&case.id, &verdict);
            }
        }
        CaseAction::PurgePhi { id } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            store.purge_case_phi(&id)?;
            let case = store
                .get_case(&id)?
                .ok_or_else(|| anyhow!("case `{id}` not found"))?;
            println!(
                "purged PHI for `{}` (retention: {})",
                case.id,
                case.raw_text_retention.as_db_str()
            );
        }
        CaseAction::PurgeAttachments { id } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            let n = store.purge_case_attachment_files(&id)?;
            println!("purged {n} raw attachment files for `{id}`");
        }
        CaseAction::Finalize {
            id,
            decision,
            reviewer,
            role,
            note,
            final_json,
        } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            let verdict = store
                .latest_verdict(&id)?
                .ok_or_else(|| anyhow!("no verdict found for case `{id}`"))?;
            let decision = ReviewDecision::from_db_str(&decision)
                .ok_or_else(|| anyhow!("decision must be accept, modify or reject"))?;
            let feedback_kind = match decision {
                ReviewDecision::Accept => FeedbackKind::Accept,
                ReviewDecision::Modify => FeedbackKind::Modify,
                ReviewDecision::Reject => FeedbackKind::Reject,
            };
            let final_verdict_json = match final_json {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?,
                ),
                None => None,
            };
            let diff_summary = final_verdict_json
                .as_ref()
                .and_then(|final_json| summarize_json_diff(&verdict.output_json, final_json));
            store.upsert_feedback(&conclave_verdict::FeedbackRecord {
                case_id: id.clone(),
                kind: feedback_kind,
                reason: note.clone(),
                modified_verdict_json: final_verdict_json.clone(),
                created_at: chrono::Utc::now(),
            })?;
            store.finalize_review(&ReviewMetadataRecord {
                case_id: id.clone(),
                verdict_id: verdict.id,
                decision,
                reviewer_name: reviewer,
                reviewer_role: role,
                note,
                final_verdict_json,
                diff_summary,
                reviewed_at: chrono::Utc::now(),
            })?;
            println!("case `{id}` finalized ({})", decision.as_db_str());
        }
        CaseAction::Workflow {
            id,
            workflow,
            terminology_dir,
            json,
        } => {
            let store = store
                .lock()
                .map_err(|_| anyhow!("case store mutex poisoned"))?;
            let case = store
                .get_case(&id)?
                .ok_or_else(|| anyhow!("case `{id}` not found"))?;
            let verdict_row = store
                .latest_verdict(&id)?
                .ok_or_else(|| anyhow!("no verdict found for case `{id}`"))?;
            let verdict: Verdict = serde_json::from_str(&verdict_row.output_json)
                .with_context(|| format!("stored verdict for `{id}` does not parse as JSON"))?;
            let workflow = ClinicalWorkflow::from_id(&workflow).ok_or_else(|| {
                anyhow!("unknown workflow; use chart_summary, med_rec_discrepancy, guideline_review, discharge_handoff, coding_audit, or structured_extraction_fhir_diff")
            })?;
            let terminology_available = match terminology_dir {
                Some(dir) => !CsvTerminologyService::from_dir(dir)?.is_empty(),
                None => false,
            };
            let out = run_workflow(workflow, &case, &verdict, terminology_available)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            } else {
                println!("{}", out.markdown);
            }
        }
    }
    Ok(())
}

async fn fetch_external_evidence(
    ctx: &CommandContext,
    query_override: Option<&str>,
    masked_text: &str,
    question: &str,
    ncbi_email: Option<&str>,
) -> Result<Vec<EvidenceItem>> {
    let query = query_override
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| build_external_evidence_query(masked_text, question));
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let cache = Arc::new(
        EvidenceCache::open(ctx.paths.cache_dir().join("evidence.sqlite"))
            .map_err(|e| anyhow!("{e}"))?,
    );
    let mut first_error: Option<String> = None;
    let email = ncbi_email
        .map(str::to_owned)
        .or_else(|| std::env::var("CONCLAVE_NCBI_EMAIL").ok());
    if let Some(email) = email {
        match PubMedSource::new(email).map(|s| s.with_cache(Arc::clone(&cache))) {
            Ok(pubmed) => match pubmed.search(&query, 5).await {
                Ok(items) if !items.is_empty() => return Ok(items),
                Ok(_) => {}
                Err(e) => first_error = Some(e.to_string()),
            },
            Err(e) => first_error = Some(e.to_string()),
        }
    }
    let europe = EuropePmcSource::new()
        .map_err(|e| anyhow!(first_error.clone().unwrap_or_else(|| e.to_string())))?
        .with_cache(cache);
    europe
        .search(&query, 5)
        .await
        .map_err(|e| anyhow!(first_error.unwrap_or_else(|| e.to_string())))
}

fn build_external_evidence_query(masked_text: &str, question: &str) -> String {
    const STOPWORDS: &[&str] = &[
        "paciente",
        "patient",
        "manejo",
        "management",
        "recomendado",
        "recommended",
        "cuál",
        "cual",
        "what",
        "with",
        "para",
        "por",
        "the",
        "and",
        "los",
        "las",
        "una",
        "uno",
        "del",
        "con",
        "sin",
        "que",
        "está",
        "esta",
        "this",
        "case",
        "años",
        "year",
        "old",
    ];
    let mut terms = Vec::new();
    let combined = format!("{question} {masked_text}");
    let mut current = String::new();
    let mut in_token = false;
    for ch in combined.chars() {
        if ch == '<' {
            current.clear();
            in_token = true;
            continue;
        }
        if in_token {
            if ch == '>' {
                in_token = false;
            }
            continue;
        }
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_query_term(&mut terms, &current, STOPWORDS);
            current.clear();
        }
        if terms.len() >= 10 {
            break;
        }
    }
    if !current.is_empty() && terms.len() < 10 {
        push_query_term(&mut terms, &current, STOPWORDS);
    }
    terms.join(" ")
}

fn push_query_term(out: &mut Vec<String>, term: &str, stopwords: &[&str]) {
    if term.len() < 4 || term.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    if stopwords.contains(&term) {
        return;
    }
    if !out.iter().any(|t| t == term) {
        out.push(term.to_owned());
    }
}

fn summarize_json_diff(original_json: &str, final_json: &str) -> Option<String> {
    if original_json.trim() == final_json.trim() {
        return None;
    }
    let Ok(original) = serde_json::from_str::<serde_json::Value>(original_json) else {
        return Some("Final verdict text differs from generated draft".to_owned());
    };
    let Ok(final_value) = serde_json::from_str::<serde_json::Value>(final_json) else {
        return Some("Final verdict text differs from generated draft".to_owned());
    };
    if original == final_value {
        return None;
    }
    let mut changed = Vec::new();
    if let (Some(o), Some(f)) = (original.as_object(), final_value.as_object()) {
        for key in f.keys() {
            if o.get(key) != f.get(key) {
                changed.push(key.clone());
            }
        }
        for key in o.keys() {
            if !f.contains_key(key) {
                changed.push(key.clone());
            }
        }
        changed.sort();
        changed.dedup();
    }
    if changed.is_empty() {
        Some("Final verdict JSON differs from generated draft".to_owned())
    } else {
        Some(format!("Changed fields: {}", changed.join(", ")))
    }
}

fn read_input(file: Option<&std::path::Path>, text: Option<&str>) -> Result<String> {
    if let Some(t) = text {
        return Ok(t.to_owned());
    }
    if let Some(p) = file {
        return std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()));
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    if buf.trim().is_empty() {
        anyhow::bail!("empty case input — pass --file, inline text or pipe via stdin");
    }
    Ok(buf)
}

pub(crate) fn case_store(
    ctx: &CommandContext,
    workspace: &conclave_core::Workspace,
) -> Result<Arc<Mutex<CaseStore>>> {
    let path = ctx.paths.workspace_dir(&workspace.id).join("cases.sqlite");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let store = CaseStore::open(&path)
        .with_context(|| format!("opening case store at {}", path.display()))?;
    Ok(Arc::new(Mutex::new(store)))
}

pub(crate) async fn build_provider(
    id: &str,
    model: Option<String>,
) -> Result<Arc<dyn LlmProvider>> {
    let api_key = match id {
        "ollama" | "anthropic-oauth" | "openai-oauth" => String::new(),
        _ => secrets::load(id)?
            .ok_or_else(|| anyhow!("no API key for {id} — run `providers set {id}`"))?,
    };
    Ok(match id {
        "anthropic" => {
            let mut p = AnthropicProvider::new(api_key);
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openai" => {
            let mut p = OpenAiProvider::new(api_key);
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openrouter" => {
            let mut p = OpenRouterProvider::new(api_key);
            p = match model {
                Some(m) => p.with_model(m),
                None => p.with_model("anthropic/claude-3.5-sonnet"),
            };
            Arc::new(p)
        }
        "ollama" => {
            let mut p = OllamaProvider::new();
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "anthropic-oauth" => {
            let mut p =
                AnthropicOAuthProvider::from_default_location().map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        "openai-oauth" => {
            let mut p = OpenAIOAuthProvider::from_default_location().map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Arc::new(p)
        }
        other => anyhow::bail!("unknown provider `{other}`"),
    })
}

fn render_pretty(case_id: &str, v: &Verdict) {
    println!("══════════════════════════════════════════════════════════════════════");
    println!("  VERDICT — case {case_id}");
    println!("══════════════════════════════════════════════════════════════════════\n");

    println!("CASE SUMMARY");
    println!("────────────");
    println!("{}\n", v.case_summary);

    if !v.key_clinical_data.is_empty() {
        println!("KEY CLINICAL DATA");
        println!("─────────────────");
        for kv in &v.key_clinical_data {
            println!("  • {}: {}", kv.label, kv.value);
        }
        println!();
    }

    println!("PRIMARY RECOMMENDATION");
    println!("──────────────────────");
    println!("  ▶ {}", v.primary_recommendation.action);
    println!("    rationale: {}\n", v.primary_recommendation.rationale);

    let certainty_marker = match v.certainty_level {
        CertaintyLevel::High => "●●●",
        CertaintyLevel::Medium => "●●○",
        CertaintyLevel::Low => "●○○",
    };
    println!(
        "CERTAINTY: {} {}",
        certainty_marker,
        v.certainty_level.label()
    );
    println!("  {}\n", v.certainty_justification);
    println!("DATA COMPLETENESS: {}\n", v.data_completeness.label());

    if !v.red_flags.is_empty() {
        println!("RED FLAGS");
        println!("─────────");
        for rf in &v.red_flags {
            println!("  ⚠ {rf}");
        }
        println!();
    }

    if !v.follow_up_triggers.is_empty() {
        println!("FOLLOW-UP TRIGGERS");
        println!("──────────────────");
        for tr in &v.follow_up_triggers {
            println!("  ↻ {tr}");
        }
        println!();
    }

    if !v.applied_evidence.is_empty() {
        println!("APPLIED EVIDENCE");
        println!("────────────────");
        for ev in &v.applied_evidence {
            println!("  [{}] {}", ev.reference, ev.claim);
        }
        println!();
    }

    println!("DISCLAIMER");
    println!("──────────");
    println!("{}", v.disclaimer);
}
