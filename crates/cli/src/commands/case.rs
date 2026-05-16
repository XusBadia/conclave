//! `conclave-cli case` — submit a clinical case to the verdict engine and
//! browse stored cases.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};

use conclave_deident::PipelineDeidentifier;
use conclave_providers::{
    secrets, AnthropicProvider, LlmProvider, OllamaProvider, OpenAiProvider, OpenRouterProvider,
};
use conclave_verdict::{CaseStore, CertaintyLevel, Verdict, VerdictOptions, VerdictPipeline};

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
            let run = pipeline
                .run(&case_text, &question, &options)
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
                println!();
                render_pretty(&case.id, &verdict);
            }
        }
    }
    Ok(())
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

fn case_store(
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

async fn build_provider(id: &str, model: Option<String>) -> Result<Arc<dyn LlmProvider>> {
    let api_key = if id == "ollama" {
        String::new()
    } else {
        secrets::load(id)?
            .ok_or_else(|| anyhow!("no API key for {id} — run `providers set {id}`"))?
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

    if !v.alternatives.is_empty() {
        println!("ALTERNATIVES");
        println!("────────────");
        for alt in &v.alternatives {
            println!("  • {} — when: {}", alt.action, alt.when_to_consider);
        }
        println!();
    }

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
