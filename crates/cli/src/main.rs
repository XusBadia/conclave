//! `conclave-cli` — terminal entry point for the Conclave virtual committee.
//!
//! Phase 0 only wires the command surface; every subcommand returns a
//! `not yet implemented` notice. Real behaviour lands in later phases.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use conclave_core::{logging, paths::Paths, Config, MEDICAL_DISCLAIMER};

mod commands;
mod embedder;

/// `conclave-cli` argument tree.
#[derive(Debug, Parser)]
#[command(
    name = "conclave-cli",
    bin_name = "conclave-cli",
    version,
    about = "Conclave — virtual multidisciplinary clinical committee (CLI)",
    long_about = "Conclave runs a virtual multidisciplinary committee over a clinical \
                  question using a configurable panel of LLM providers.\n\n\
                  This binary is the testing entry point used while the desktop UI is \
                  under construction."
)]
struct Cli {
    /// Override the workspace root (defaults to the OS-standard app dirs).
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        env = "CONCLAVE_WORKSPACE_ROOT"
    )]
    workspace_root: Option<PathBuf>,

    /// Suppress the medical disclaimer printed on every invocation.
    #[arg(long, global = true)]
    no_disclaimer: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Ingest documents into the knowledge base.
    Ingest(commands::ingest::IngestArgs),
    /// Search the knowledge base.
    Search(commands::search::SearchArgs),
    /// Run a virtual committee and print its verdict.
    Verdict(commands::verdict::VerdictArgs),
    /// Inspect, list and test configured LLM providers.
    Providers(commands::providers::ProvidersArgs),
    /// Manage Conclave workspaces (the per-project config + data root).
    Workspace(commands::workspace::WorkspaceArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let paths = resolve_paths(cli.workspace_root.as_deref())?;
    paths.ensure_exists()?;
    let config = Config::load(paths.config_file())?;

    logging::init(config.general.log_format);

    if !cli.no_disclaimer {
        print_disclaimer();
    }

    tracing::debug!(
        config_dir = %paths.config_dir().display(),
        data_dir = %paths.data_dir().display(),
        cache_dir = %paths.cache_dir().display(),
        "resolved workspace paths"
    );

    let ctx = commands::CommandContext { paths, config };

    match cli.command {
        Command::Ingest(args) => commands::ingest::run(&ctx, args),
        Command::Search(args) => commands::search::run(&ctx, args),
        Command::Verdict(args) => commands::verdict::run(&ctx, args),
        Command::Providers(args) => commands::providers::run(&ctx, args),
        Command::Workspace(args) => commands::workspace::run(&ctx, args),
    }
}

fn resolve_paths(workspace_root: Option<&std::path::Path>) -> Result<Paths> {
    Ok(match workspace_root {
        Some(root) => Paths::rooted_at(root),
        None => Paths::resolve()?,
    })
}

fn print_disclaimer() {
    eprintln!("──────────────────────────────────────────────────────────────────────");
    eprintln!("  CONCLAVE — MEDICAL DISCLAIMER");
    eprintln!("──────────────────────────────────────────────────────────────────────");
    for line in wrap_paragraph(MEDICAL_DISCLAIMER, 70) {
        eprintln!("  {line}");
    }
    eprintln!("──────────────────────────────────────────────────────────────────────");
}

fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_lists_all_subcommands() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        for sub in ["ingest", "verdict", "providers", "workspace"] {
            assert!(
                help.contains(sub),
                "help missing subcommand `{sub}`:\n{help}"
            );
        }
    }

    #[test]
    fn wrap_paragraph_respects_width() {
        let lines = wrap_paragraph(MEDICAL_DISCLAIMER, 70);
        for line in &lines {
            assert!(line.len() <= 70, "line too long: {line:?}");
        }
        assert!(!lines.is_empty());
    }
}
