//! Always-on, clinician-curated domain priors injected into the prompt's
//! `WORKSPACE RULES` block by workspace specialty.
//!
//! Unlike skills (one active at a time, user-selected), priors are matched
//! automatically and injected on every run for the matching specialty. Because
//! the prompt frames `WORKSPACE RULES` as constraints that invalidate the
//! response when violated, domain safety rules here cannot be silently dropped
//! or overridden by a generic model heuristic. Content is bundled at compile
//! time, versioned in-repo, conservative, and clinician-reviewed.

/// Bundled colorectal ruleset.
const COLORECTAL: &str = include_str!("priors/colorectal.md");

/// Return the curated domain priors for a workspace specialty, if any.
///
/// Matching is a conservative case-insensitive substring test on the specialty
/// label, so "colorrectal", "oncología colorrectal", "cirugía colorrectal" and
/// "coloproctología" all resolve to the colorectal ruleset.
#[must_use]
pub fn load_specialty_priors(specialty: &str) -> Option<&'static str> {
    let s = specialty.to_lowercase();
    if s.contains("colorrectal") || s.contains("colorectal") || s.contains("coloproctolog") {
        return Some(COLORECTAL);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorectal_specialty_matches() {
        for s in [
            "colorrectal",
            "Oncología colorrectal",
            "Cirugía colorrectal",
            "Coloproctología",
            "colorectal surgery",
        ] {
            let priors = load_specialty_priors(s).unwrap_or_else(|| panic!("no priors for {s:?}"));
            assert!(priors.contains("Watch & Wait"));
        }
    }

    #[test]
    fn unrelated_specialty_has_no_priors() {
        assert!(load_specialty_priors("cardiología").is_none());
        assert!(load_specialty_priors("medicina general").is_none());
    }

    #[test]
    fn colorectal_priors_cover_the_regrowth_rule() {
        let priors = load_specialty_priors("colorrectal").unwrap();
        // The W&W biopsy-negative-regrowth rule must be present and stated as
        // a general principle, not overfit to one case.
        assert!(priors.contains("negative biopsy does NOT rule out local regrowth"));
        assert!(priors.contains("MMR/MSI"));
    }
}
