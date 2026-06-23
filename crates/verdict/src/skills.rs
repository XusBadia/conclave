//! Versionable markdown skills used as prompt overlays.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

use crate::privacy::DataBoundaryMode;

/// A Conclave skill loaded from built-ins, user config, or a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub recommended_workflow: String,
    pub allowed_modes: Vec<String>,
    pub body: String,
    pub source: SkillSource,
}

impl Skill {
    /// Whether this skill may run under the selected data-boundary mode.
    /// Empty `allowed_modes` means unrestricted, which keeps custom skills
    /// backward-compatible while built-ins can be stricter.
    pub fn allows_mode(&self, mode: DataBoundaryMode) -> bool {
        self.allowed_modes.is_empty() || self.allowed_modes.iter().any(|m| m == mode.as_db_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    BuiltIn,
    User,
    Workspace,
}

/// Load skills with precedence workspace > user > built-in.
pub fn load_skills(user_dir: Option<&Path>, workspace_dir: Option<&Path>) -> Result<Vec<Skill>> {
    let mut by_id = BTreeMap::new();
    for skill in built_in_skills()? {
        by_id.insert(skill.id.clone(), skill);
    }
    if let Some(dir) = user_dir {
        for skill in load_dir(dir, SkillSource::User)? {
            by_id.insert(skill.id.clone(), skill);
        }
    }
    if let Some(dir) = workspace_dir {
        for skill in load_dir(dir, SkillSource::Workspace)? {
            by_id.insert(skill.id.clone(), skill);
        }
    }
    Ok(by_id.into_values().collect())
}

/// Fetch one skill by id after applying normal precedence.
pub fn load_skill(
    id: &str,
    user_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
) -> Result<Option<Skill>> {
    Ok(load_skills(user_dir, workspace_dir)?
        .into_iter()
        .find(|s| s.id == id))
}

fn built_in_skills() -> Result<Vec<Skill>> {
    const RAW: &[(&str, &str)] = &[
        (
            "tumor-board",
            r"---
id: tumor-board
title: Tumor board
description: Multidisciplinary oncology review with staging, certainty, red flags, and follow-up triggers.
recommended_workflow: guideline_review
allowed_modes: deid_cloud,explicit_phi
---
Frame the answer as a multidisciplinary oncology board. Commit to a single management recommendation; explicitly separate staging assumptions, missing pathology/imaging data, treatment intent, red flags, and follow-up triggers.
",
        ),
        (
            "emergency-review",
            r"---
id: emergency-review
title: Emergency review
description: Safety-first acute-care review focused on red flags and escalation.
recommended_workflow: chart_summary
allowed_modes: local_only,deid_cloud,explicit_phi
---
Prioritize immediate safety, escalation criteria, differential diagnoses that cannot be missed, and what data must be obtained before discharge or de-escalation.
",
        ),
        (
            "guideline-grounded",
            r"---
id: guideline-grounded
title: Guideline grounded
description: Strict review against local protocols and cited evidence.
recommended_workflow: guideline_review
allowed_modes: local_only,deid_cloud
---
Use local evidence first. If the evidence is missing or conflicting, say so plainly and lower certainty. Do not rely on unstated general knowledge for the primary recommendation.
",
        ),
        (
            "patient-message-draft",
            r"---
id: patient-message-draft
title: Patient message draft
description: Reviewer-safe patient-facing communication.
recommended_workflow: discharge_handoff
allowed_modes: local_only,deid_cloud,explicit_phi
---
Write patient-facing language only as a draft for clinician review. Avoid definitive diagnosis language unless already established in the supplied evidence.
",
        ),
        (
            "literature-review",
            r"---
id: literature-review
title: Literature review
description: Evidence synthesis with clear separation of local and external sources.
recommended_workflow: chart_summary
allowed_modes: deid_cloud
---
Separate local protocol evidence from external literature. Label external evidence as not validated by the centre and avoid changing recommendations unless evidence quality is strong.
",
        ),
        (
            "documentation-note",
            r"---
id: documentation-note
title: Documentation note
description: Structured clinical documentation support.
recommended_workflow: chart_summary
allowed_modes: local_only,deid_cloud
---
Focus on concise, structured clinical documentation: relevant positives/negatives, assessment, plan, uncertainty, and follow-up. Do not invent exam findings or vitals.
",
        ),
    ];
    RAW.iter()
        .map(|(_, raw)| parse_skill(raw, SkillSource::BuiltIn))
        .collect()
}

fn load_dir(dir: &Path, source: SkillSource) -> Result<Vec<Skill>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| Error::invalid_config(e.to_string()))? {
        let entry = entry.map_err(|e| Error::invalid_config(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            let skill_path = path.join("SKILL.md");
            if skill_path.exists() {
                out.push(parse_file(&skill_path, source)?);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(parse_file(&path, source)?);
        }
    }
    Ok(out)
}

fn parse_file(path: &Path, source: SkillSource) -> Result<Skill> {
    let raw = fs::read_to_string(path)
        .map_err(|e| Error::invalid_config(format!("reading skill {}: {e}", path.display())))?;
    parse_skill(&raw, source).map_err(|e| {
        Error::invalid_config(format!(
            "invalid skill {}: {e}",
            PathBuf::from(path).display()
        ))
    })
}

fn parse_skill(raw: &str, source: SkillSource) -> Result<Skill> {
    let trimmed = raw.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return Err(Error::invalid_config("skill missing frontmatter"));
    };
    let Some((frontmatter, body)) = rest.split_once("---") else {
        return Err(Error::invalid_config("skill frontmatter is not closed"));
    };
    let mut fields = BTreeMap::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(Error::invalid_config(format!(
                "invalid frontmatter line `{line}`"
            )));
        };
        fields.insert(
            key.trim().to_owned(),
            value.trim().trim_matches('"').to_owned(),
        );
    }
    let required = |key: &str| -> Result<String> {
        fields
            .get(key)
            .cloned()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| Error::invalid_config(format!("skill missing `{key}`")))
    };
    let allowed_modes = fields
        .get("allowed_modes")
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(Skill {
        id: required("id")?,
        title: required("title")?,
        description: required("description")?,
        recommended_workflow: fields
            .get("recommended_workflow")
            .cloned()
            .unwrap_or_else(|| "chart_summary".into()),
        allowed_modes,
        body: body.trim().to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_parse() {
        let skills = built_in_skills().unwrap();
        assert_eq!(skills.len(), 6);
        assert!(skills.iter().any(|s| s.id == "tumor-board"));
    }

    #[test]
    fn workspace_overrides_user_and_builtin() {
        let user = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            user.path().join("tumor-board.md"),
            r"---
id: tumor-board
title: User board
description: User override
recommended_workflow: chart_summary
allowed_modes: local_only
---
user body
",
        )
        .unwrap();
        fs::write(
            workspace.path().join("tumor-board.md"),
            r"---
id: tumor-board
title: Workspace board
description: Workspace override
recommended_workflow: guideline_review
allowed_modes: deid_cloud
---
workspace body
",
        )
        .unwrap();

        let skill = load_skill("tumor-board", Some(user.path()), Some(workspace.path()))
            .unwrap()
            .unwrap();
        assert_eq!(skill.source, SkillSource::Workspace);
        assert_eq!(skill.title, "Workspace board");
        assert!(skill.allows_mode(DataBoundaryMode::DeidCloud));
        assert!(!skill.allows_mode(DataBoundaryMode::LocalOnly));
    }

    #[test]
    fn invalid_markdown_skill_fails_validation() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.md"), "missing frontmatter").unwrap();
        assert!(load_skills(None, Some(dir.path())).is_err());
    }
}
