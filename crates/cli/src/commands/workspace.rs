//! `conclave-cli workspace` — manage Conclave workspaces.

use anyhow::Result;
use clap::{Args, Subcommand};

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
    /// Print the resolved paths and active configuration summary.
    Info,
    /// Initialise the on-disk config file with defaults if it is missing.
    Init {
        /// Overwrite an existing config file.
        #[arg(long)]
        force: bool,
    },
}

/// Execute the `workspace` subcommand.
pub(crate) fn run(ctx: &CommandContext, args: WorkspaceArgs) -> Result<()> {
    match args.action {
        WorkspaceAction::Info => {
            println!("config dir: {}", ctx.paths.config_dir().display());
            println!("data dir:   {}", ctx.paths.data_dir().display());
            println!("cache dir:  {}", ctx.paths.cache_dir().display());
            println!("config file: {}", ctx.paths.config_file().display());
            println!(
                "default workspace: {}",
                ctx.config.general.default_workspace
            );
            println!(
                "default provider: {}",
                ctx.config.providers.default.as_deref().unwrap_or("<unset>"),
            );
        }
        WorkspaceAction::Init { force } => {
            let path = ctx.paths.config_file();
            if path.exists() && !force {
                anyhow::bail!(
                    "config file already exists at {} — pass --force to overwrite",
                    path.display(),
                );
            }
            ctx.config.save(&path)?;
            println!("wrote default config to {}", path.display());
        }
    }
    Ok(())
}
