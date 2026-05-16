//! `conclave-cli feedback` — accept / modify / reject a previously
//! generated case verdict.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};

use conclave_verdict::persistence::{FeedbackKind, FeedbackRecord};
use conclave_verdict::CaseStore;

use super::CommandContext;

/// Arguments for the `feedback` subcommand.
#[derive(Debug, Args)]
pub(crate) struct FeedbackArgs {
    #[command(subcommand)]
    action: FeedbackAction,
}

#[derive(Debug, Subcommand)]
enum FeedbackAction {
    /// Mark the case as accepted.
    Accept {
        case_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Replace the stored verdict with a user-modified version.
    Modify {
        case_id: String,
        /// Path to the modified verdict JSON.
        #[arg(long, value_name = "PATH")]
        from: PathBuf,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Mark the case as rejected.
    Reject {
        case_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

pub(crate) fn run(ctx: &CommandContext, args: FeedbackArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let path = ctx.paths.workspace_dir(&workspace.id).join("cases.sqlite");
    let store = CaseStore::open(&path)
        .with_context(|| format!("opening case store at {}", path.display()))?;
    let store = Arc::new(Mutex::new(store));

    let (case_id, kind, reason, modified) = match args.action {
        FeedbackAction::Accept { case_id, reason } => (case_id, FeedbackKind::Accept, reason, None),
        FeedbackAction::Reject { case_id, reason } => (case_id, FeedbackKind::Reject, reason, None),
        FeedbackAction::Modify {
            case_id,
            from,
            reason,
        } => {
            let modified = std::fs::read_to_string(&from)
                .with_context(|| format!("reading {}", from.display()))?;
            // Sanity-parse so we don't store garbage.
            let _: serde_json::Value = serde_json::from_str(&modified)
                .with_context(|| format!("{} is not valid JSON", from.display()))?;
            (case_id, FeedbackKind::Modify, reason, Some(modified))
        }
    };

    let record = FeedbackRecord {
        case_id: case_id.clone(),
        kind,
        reason,
        modified_verdict_json: modified,
        created_at: Utc::now(),
    };

    {
        let store = store
            .lock()
            .map_err(|_| anyhow!("case store mutex poisoned"))?;
        store.upsert_feedback(&record)?;
    }

    let label = match kind {
        FeedbackKind::Accept => "accept",
        FeedbackKind::Modify => "modify",
        FeedbackKind::Reject => "reject",
    };
    println!("feedback `{label}` recorded for case `{case_id}`");
    Ok(())
}
