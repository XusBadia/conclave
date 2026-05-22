//! `conclave-cli audit` — inspect local fingerprint-first audit runs.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use conclave_verdict::CaseStore;

use super::CommandContext;

#[derive(Debug, Args)]
pub(crate) struct AuditArgs {
    #[command(subcommand)]
    action: AuditAction,
}

#[derive(Debug, Subcommand)]
enum AuditAction {
    /// Show audit counters for the active workspace.
    Status,
    /// List recent audit runs.
    Show {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Export audit runs as JSON.
    Export,
    /// Verify basic audit-store consistency.
    Verify,
    /// Clear stale raw text already marked as discarded.
    Cleanup,
}

pub(crate) fn run(ctx: &CommandContext, args: AuditArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let path = ctx.paths.workspace_dir(&workspace.id).join("cases.sqlite");
    let store = CaseStore::open(&path)
        .with_context(|| format!("opening case store at {}", path.display()))?;
    match args.action {
        AuditAction::Status => {
            let status = store.audit_status()?;
            println!("audit runs:       {}", status.run_count);
            println!("payload mode:     {}", status.payload_mode.as_db_str());
            println!("raw retained:     {}", status.retained_raw_cases);
            println!("legacy retained:  {}", status.legacy_retained_cases);
        }
        AuditAction::Show { limit, json } => {
            let runs = store.list_audit_runs(limit)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&runs).unwrap_or_default()
                );
            } else if runs.is_empty() {
                println!("(no audit runs)");
            } else {
                for run in runs {
                    println!(
                        "{}  {}  {}  {}  {}",
                        run.started_at.format("%Y-%m-%d %H:%M:%S"),
                        run.id,
                        run.case_id,
                        run.provider_id,
                        run.status
                    );
                }
            }
        }
        AuditAction::Export => {
            let runs = store.list_audit_runs(usize::MAX)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&runs).unwrap_or_default()
            );
        }
        AuditAction::Verify => {
            let status = store.audit_status()?;
            if status.legacy_retained_cases > 0 {
                println!(
                    "ok with warning: {} legacy cases still retain raw text",
                    status.legacy_retained_cases
                );
            } else {
                println!("ok");
            }
        }
        AuditAction::Cleanup => {
            let n = store.cleanup_discarded_phi()?;
            println!("cleaned {n} discarded raw-text rows");
        }
    }
    Ok(())
}
