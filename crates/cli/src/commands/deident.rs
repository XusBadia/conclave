//! `conclave-cli deident` — run the Phase 3 de-identification pipeline
//! over text or a file.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use conclave_deident::{Deidentifier, PipelineDeidentifier};

use super::CommandContext;

/// Arguments for the `deident` subcommand.
#[derive(Debug, Args)]
pub(crate) struct DeidentArgs {
    /// Inline text. If absent, reads from `--file` or stdin.
    text: Option<String>,
    /// Read input from this file instead of `text`.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// Fail with a non-zero exit code if `strict_mode_clean` is false.
    #[arg(long)]
    strict: bool,
    /// Print only the masked text (skips the JSON envelope).
    #[arg(long)]
    text_only: bool,
}

pub(crate) fn run(_ctx: &CommandContext, args: DeidentArgs) -> Result<()> {
    let input = read_input(&args)?;
    let pipeline = PipelineDeidentifier::new();
    let result = pipeline.deidentify(&input)?;

    if args.text_only {
        println!("{}", result.masked_text);
    } else {
        let json =
            serde_json::to_string_pretty(&result).context("serialising deident result as JSON")?;
        println!("{json}");
    }

    if args.strict && !result.strict_mode_clean {
        anyhow::bail!("strict mode: residual suspicious patterns found in masked output");
    }
    Ok(())
}

fn read_input(args: &DeidentArgs) -> Result<String> {
    if let Some(t) = &args.text {
        return Ok(t.clone());
    }
    if let Some(p) = &args.file {
        return std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()));
    }
    // Fall back to stdin.
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok(buf)
}
