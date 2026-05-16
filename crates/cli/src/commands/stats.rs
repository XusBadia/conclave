//! `conclave-cli stats` — counts, rates and latencies over a workspace.

use anyhow::{Context, Result};
use clap::Args;

use conclave_verdict::CaseStore;

use super::CommandContext;

/// Arguments for the `stats` subcommand.
#[derive(Debug, Args)]
pub(crate) struct StatsArgs {
    /// Print machine-readable JSON instead of a pretty summary.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn run(ctx: &CommandContext, args: StatsArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let path = ctx.paths.workspace_dir(&workspace.id).join("cases.sqlite");
    let store = CaseStore::open(&path)
        .with_context(|| format!("opening case store at {}", path.display()))?;
    let stats = store.stats()?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&stats).unwrap_or_default()
        );
        return Ok(());
    }

    let total = stats.total_cases;
    println!("workspace: {} ({})\n", workspace.name, workspace.id);
    println!("Total cases:    {total}");
    println!(
        "  completed:    {} ({:.0}%)",
        stats.completed,
        pct(stats.completed, total)
    );
    println!(
        "  failed:       {} ({:.0}%)",
        stats.failed,
        pct(stats.failed, total)
    );

    println!("\nFeedback:");
    let acc = *stats.feedback_counts.get("accept").unwrap_or(&0);
    let modi = *stats.feedback_counts.get("modify").unwrap_or(&0);
    let rej = *stats.feedback_counts.get("reject").unwrap_or(&0);
    let total_fb = acc + modi + rej;
    let no_fb = total.saturating_sub(total_fb);
    println!("  accepted:     {acc} ({:.0}%)", pct(acc, total));
    println!("  modified:     {modi} ({:.0}%)", pct(modi, total));
    println!("  rejected:     {rej} ({:.0}%)", pct(rej, total));
    println!("  no feedback:  {no_fb}");

    println!(
        "\nAverage verdict latency: {}",
        stats
            .avg_latency_ms
            .map_or("—".to_string(), |v| format!("{v:.0}ms"))
    );
    println!("Cases in last 7 days:    {}", stats.cases_last_7d);
    Ok(())
}

fn pct(n: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (n as f64) * 100.0 / (total as f64)
    }
}
