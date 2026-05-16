//! `conclave-cli export dataset` — dump every case + verdict + feedback
//! from the active workspace as JSON. Uses masked text only; original
//! patient text is never written.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use conclave_verdict::CaseStore;

use super::CommandContext;

/// Arguments for the `export` subcommand.
#[derive(Debug, Args)]
pub(crate) struct ExportArgs {
    #[command(subcommand)]
    action: ExportAction,
}

#[derive(Debug, Subcommand)]
enum ExportAction {
    /// Dump the workspace's case/verdict/feedback rows.
    Dataset {
        /// Output file (default: stdout).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
}

pub(crate) fn run(ctx: &CommandContext, args: ExportArgs) -> Result<()> {
    let ExportAction::Dataset { out } = args.action;
    let workspace = ctx.resolve_workspace(None)?;
    let path = ctx.paths.workspace_dir(&workspace.id).join("cases.sqlite");
    let store = CaseStore::open(&path)
        .with_context(|| format!("opening case store at {}", path.display()))?;
    let rows = store.export()?;
    let payload = serde_json::json!({
        "workspace_id": workspace.id,
        "workspace_name": workspace.name,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "count": rows.len(),
        "cases": rows,
    });
    let pretty = serde_json::to_string_pretty(&payload).unwrap_or_default();
    if let Some(p) = out {
        std::fs::write(&p, &pretty).with_context(|| format!("writing {}", p.display()))?;
        println!("wrote {} cases to {}", rows.len(), p.display());
    } else {
        println!("{pretty}");
    }
    Ok(())
}
