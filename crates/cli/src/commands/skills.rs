//! `conclave-cli skills` — inspect versionable markdown prompt overlays.

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use conclave_verdict::{load_skill, load_skills};

use super::CommandContext;

#[derive(Debug, Args)]
pub(crate) struct SkillsArgs {
    #[command(subcommand)]
    action: SkillsAction,
}

#[derive(Debug, Subcommand)]
enum SkillsAction {
    /// List built-in, user, and workspace skills after precedence is applied.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one skill body.
    Show { id: String },
    /// Parse every available skill and fail on invalid frontmatter.
    Validate,
}

pub(crate) fn run(ctx: &CommandContext, args: SkillsArgs) -> Result<()> {
    let workspace = ctx.resolve_workspace(None)?;
    let user_skills = ctx.paths.config_dir().join("skills");
    let workspace_skills = ctx.paths.workspace_dir(&workspace.id).join("skills");
    match args.action {
        SkillsAction::List { json } => {
            let skills = load_skills(Some(&user_skills), Some(&workspace_skills))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&skills).unwrap_or_default()
                );
            } else {
                for s in skills {
                    println!(
                        "{:<24} {:<12} {}",
                        s.id,
                        format!("{:?}", s.source).to_lowercase(),
                        s.title
                    );
                }
            }
        }
        SkillsAction::Show { id } => {
            let skill = load_skill(&id, Some(&user_skills), Some(&workspace_skills))?
                .ok_or_else(|| anyhow!("skill `{id}` not found"))?;
            println!("# {}\n", skill.title);
            println!("{}", skill.description);
            println!("\nworkflow: {}", skill.recommended_workflow);
            println!("source:   {:?}", skill.source);
            println!("\n{}", skill.body);
        }
        SkillsAction::Validate => {
            let skills = load_skills(Some(&user_skills), Some(&workspace_skills))?;
            println!("ok: {} skills", skills.len());
        }
    }
    Ok(())
}
