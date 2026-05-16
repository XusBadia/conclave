//! `conclave-cli workspace` — manage Conclave workspaces.

use anyhow::Result;
use clap::{Args, Subcommand};

use conclave_core::WorkspaceManager;

use super::CommandContext;

/// Arguments for the `workspace` subcommand.
#[derive(Debug, Args)]
pub(crate) struct WorkspaceArgs {
    #[command(subcommand)]
    pub(crate) action: WorkspaceAction,
}

/// Sub-actions exposed by `workspace`.
#[derive(Debug, Subcommand)]
pub(crate) enum WorkspaceAction {
    /// Create a new workspace under the data root.
    Create {
        /// Human-friendly workspace name (used to derive a slug id).
        name: String,
        /// Optional clinical specialty (e.g. `cardiology`).
        #[arg(long)]
        specialty: Option<String>,
        /// Optional default language ISO code (e.g. `es`, `en`).
        #[arg(long)]
        language: Option<String>,
    },
    /// List every workspace found under the data root.
    List,
    /// Set the active workspace (persisted to the global config).
    Switch {
        /// Workspace id or name to make active.
        name: String,
    },
    /// Delete a workspace and every file inside its directory.
    Delete {
        /// Workspace id or name to delete.
        name: String,
        /// Required confirmation: deletion is unrecoverable.
        #[arg(long)]
        confirm: bool,
    },
    /// Print the resolved paths and active configuration summary.
    Info,
}

/// Execute the `workspace` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: WorkspaceArgs) -> Result<()> {
    let manager = WorkspaceManager::new(ctx.paths.workspaces_dir());
    match args.action {
        WorkspaceAction::Create {
            name,
            specialty,
            language,
        } => {
            let ws = manager.create(&name, specialty, language)?;
            let dir = ctx.paths.workspace_dir(&ws.id);
            println!("created workspace `{}` (id: {})", ws.name, ws.id);
            println!("  path: {}", dir.display());
            if let Some(s) = ws.specialty.as_deref() {
                println!("  specialty: {s}");
            }
            if let Some(l) = ws.language.as_deref() {
                println!("  language:  {l}");
            }
        }
        WorkspaceAction::List => {
            let list = manager.list()?;
            if list.is_empty() {
                println!(
                    "(no workspaces yet — create one with `conclave-cli workspace create <name>`)"
                );
            } else {
                for ws in list {
                    let active = if ws.id == ctx.config.general.default_workspace {
                        "* "
                    } else {
                        "  "
                    };
                    let specialty = ws
                        .specialty
                        .as_deref()
                        .map(|s| format!(" [{s}]"))
                        .unwrap_or_default();
                    let language = ws
                        .language
                        .as_deref()
                        .map(|l| format!(" ({l})"))
                        .unwrap_or_default();
                    println!("{active}{:<28} {}{specialty}{language}", ws.id, ws.name);
                }
            }
        }
        WorkspaceAction::Switch { name } => {
            let ws = manager.load(&name)?;
            let mut new_config = ctx.config.clone();
            new_config.general.default_workspace.clone_from(&ws.id);
            new_config.save(ctx.paths.config_file())?;
            println!("active workspace: {} ({})", ws.name, ws.id);
        }
        WorkspaceAction::Delete { name, confirm } => {
            if !confirm {
                anyhow::bail!(
                    "refusing to delete workspace `{name}` without --confirm (this is unrecoverable)"
                );
            }
            let ws = manager.load(&name)?;
            manager.delete(&ws.id)?;
            println!("deleted workspace `{}` ({})", ws.name, ws.id);
        }
        WorkspaceAction::Info => {
            println!("config dir:     {}", ctx.paths.config_dir().display());
            println!("data dir:       {}", ctx.paths.data_dir().display());
            println!("cache dir:      {}", ctx.paths.cache_dir().display());
            println!("config file:    {}", ctx.paths.config_file().display());
            println!("workspaces dir: {}", ctx.paths.workspaces_dir().display());
            println!(
                "default workspace: {}",
                ctx.config.general.default_workspace
            );
            println!(
                "default provider:  {}",
                ctx.config.providers.default.as_deref().unwrap_or("<unset>"),
            );
        }
    }
    Ok(())
}
