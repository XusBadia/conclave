//! Reproducible concordance scoring for batch validation against a known
//! decision (e.g. a tumour-board "Pla d'actuació").
//!
//! Pure and deterministic: the same corpus scores identically across runs, so
//! each pipeline change can be measured — especially **concordance stratified
//! by certainty**, which is the calibration signal (do high-certainty cases
//! concord more than low-certainty ones?).
//!
//! [`DecisionCategory::classify`] is a conservative keyword heuristic over the
//! free-text recommendation; it is an aid, not an oracle. Callers should always
//! surface the raw action text so a clinician can override a misclassification.

use serde::{Deserialize, Serialize};

use crate::schema::{CertaintyLevel, DataCompleteness, Verdict};

/// Normalised clinical decision category for colorectal tumour-board cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCategory {
    /// Operative management (resection, TME, colectomy, …).
    Surgery,
    /// Pre-operative chemoradiotherapy or total neoadjuvant therapy.
    NeoadjuvantTherapy,
    /// Post-operative adjuvant chemotherapy.
    AdjuvantTherapy,
    /// Palliative/metastatic systemic therapy (chemo, targeted, immunotherapy).
    SystemicTherapy,
    /// Active surveillance / scheduled follow-up.
    Surveillance,
    /// Organ-preserving Watch & Wait after a complete clinical response.
    WatchAndWait,
    /// Complete staging / workup before committing to a path.
    FurtherStaging,
    /// Best supportive / symptom-directed care.
    Palliative,
    /// Nothing matched — surface the raw action for manual classification.
    Other,
}

impl DecisionCategory {
    /// Conservative keyword classifier over a free-text primary recommendation
    /// (Spanish/English). Returns [`DecisionCategory::Other`] when nothing
    /// matches. Order is most-specific-first; ambiguous multi-intent actions
    /// resolve to the earliest match, so always show the raw text too.
    #[must_use]
    pub fn classify(action: &str) -> Self {
        let a = action.to_lowercase();
        if contains_any(
            &a,
            &[
                "paliativ",
                "palliative",
                "best supportive",
                "soporte sintom",
                "confort",
                "symptom-directed",
            ],
        ) {
            Self::Palliative
        } else if contains_any(
            &a,
            &[
                "neoadyuvan",
                "neoadjuvant",
                "quimiorradio",
                "chemoradio",
                "tnt",
                "preoperator",
            ],
        ) {
            Self::NeoadjuvantTherapy
        } else if contains_any(
            &a,
            &[
                "watch & wait",
                "watch and wait",
                "w&w",
                "espera vigilante",
                "preservación de órgano",
                "preservacion de organo",
                "organ preserv",
            ],
        ) {
            Self::WatchAndWait
        } else if contains_any(&a, &["adyuvan", "adjuvant"]) {
            Self::AdjuvantTherapy
        } else if contains_any(
            &a,
            &[
                "quimioterap",
                "chemotherap",
                "folfox",
                "capox",
                "folfiri",
                "sistémic",
                "sistemic",
                "systemic",
                "inmunoterap",
                "immunotherap",
                "terapia diana",
                "targeted therapy",
            ],
        ) {
            Self::SystemicTherapy
        } else if contains_any(
            &a,
            &[
                "cirug",
                "surgery",
                "surgical",
                "resec",
                "mesorectal",
                "mesorrectal",
                "colectom",
                "hartmann",
                "exéresis",
                "exeresis",
                "escisión total",
                "total mesorectal",
            ],
        ) {
            Self::Surgery
        } else if contains_any(
            &a,
            &[
                "estadific",
                "staging",
                "reestadific",
                "completar el estudio",
                "completar estudio",
                "complete staging",
                "complete the workup",
                "biopsia",
                "biopsy",
                "mmr",
                "msi",
                "resonancia",
                "colonoscop",
                "pet-tc",
                "pet/ct",
            ],
        ) {
            Self::FurtherStaging
        } else if contains_any(
            &a,
            &[
                "seguimiento",
                "vigilancia",
                "surveillance",
                "follow-up",
                "follow up",
                "control evolutiv",
            ],
        ) {
            Self::Surveillance
        } else {
            Self::Other
        }
    }
}

/// Three-level concordance between an expected and a predicted decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Concordance {
    /// Same decision category.
    Concordant,
    /// Clinically adjacent categories (e.g. surveillance vs. watch & wait).
    Partial,
    /// Materially different decisions.
    Discordant,
}

impl Concordance {
    /// Score one expected/predicted pair.
    #[must_use]
    pub fn score(expected: DecisionCategory, predicted: DecisionCategory) -> Self {
        if expected == predicted {
            Self::Concordant
        } else if are_related(expected, predicted) {
            Self::Partial
        } else {
            Self::Discordant
        }
    }
}

/// Whether two distinct categories are clinically adjacent (→ partial credit).
fn are_related(a: DecisionCategory, b: DecisionCategory) -> bool {
    use DecisionCategory::{
        FurtherStaging, NeoadjuvantTherapy, Palliative, Surgery, Surveillance, SystemicTherapy,
        WatchAndWait,
    };
    let related = |x, y| matches!((a, b), (p, q) if (p, q) == (x, y) || (p, q) == (y, x));
    related(Surveillance, WatchAndWait)
        || related(Surveillance, FurtherStaging)
        || related(WatchAndWait, FurtherStaging)
        || related(Surgery, NeoadjuvantTherapy)
        || related(Surgery, FurtherStaging)
        || related(NeoadjuvantTherapy, SystemicTherapy)
        || related(SystemicTherapy, Palliative)
}

/// One scored case in a concordance run.
#[derive(Debug, Clone, Serialize)]
pub struct CaseOutcome {
    pub id: String,
    pub expected: DecisionCategory,
    pub predicted: DecisionCategory,
    pub concordance: Concordance,
    pub certainty: CertaintyLevel,
    pub data_completeness: DataCompleteness,
    /// Raw recommendation text, kept for manual review/override.
    pub action: String,
}

impl CaseOutcome {
    /// Score a verdict against an expected decision category.
    #[must_use]
    pub fn score(id: String, expected: DecisionCategory, verdict: &Verdict) -> Self {
        let action = verdict.primary_recommendation.action.clone();
        let predicted = DecisionCategory::classify(&action);
        Self {
            id,
            expected,
            predicted,
            concordance: Concordance::score(expected, predicted),
            certainty: verdict.certainty_level,
            data_completeness: verdict.data_completeness,
            action,
        }
    }
}

/// Concordant/partial/discordant counts for a set of cases.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Tally {
    pub concordant: usize,
    pub partial: usize,
    pub discordant: usize,
}

impl Tally {
    fn add(&mut self, c: Concordance) {
        match c {
            Concordance::Concordant => self.concordant += 1,
            Concordance::Partial => self.partial += 1,
            Concordance::Discordant => self.discordant += 1,
        }
    }

    /// Total cases tallied.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.concordant + self.partial + self.discordant
    }

    /// Strict concordance rate (`concordant / total`) in `[0, 1]`, or `None`
    /// when the stratum is empty.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn strict_rate(&self) -> Option<f32> {
        let total = self.total();
        (total > 0).then(|| self.concordant as f32 / total as f32)
    }
}

/// Concordance stratified by the verdict's certainty level — the calibration
/// view. Well-calibrated confidence implies `high ≥ medium ≥ low`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct StratifiedConcordance {
    pub high: Tally,
    pub medium: Tally,
    pub low: Tally,
}

/// Aggregate report for a concordance run.
#[derive(Debug, Clone, Serialize)]
pub struct ConcordanceReport {
    pub overall: Tally,
    pub by_certainty: StratifiedConcordance,
    pub outcomes: Vec<CaseOutcome>,
}

impl ConcordanceReport {
    /// Build a report from scored case outcomes.
    #[must_use]
    pub fn from_outcomes(outcomes: Vec<CaseOutcome>) -> Self {
        let mut overall = Tally::default();
        let mut by_certainty = StratifiedConcordance::default();
        for o in &outcomes {
            overall.add(o.concordance);
            let stratum = match o.certainty {
                CertaintyLevel::High => &mut by_certainty.high,
                CertaintyLevel::Medium => &mut by_certainty.medium,
                CertaintyLevel::Low => &mut by_certainty.low,
            };
            stratum.add(o.concordance);
        }
        Self {
            overall,
            by_certainty,
            outcomes,
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{KeyValue, Recommendation};

    fn verdict_with(action: &str, certainty: CertaintyLevel) -> Verdict {
        Verdict {
            case_summary: "s".into(),
            key_clinical_data: Vec::<KeyValue>::new(),
            applied_evidence: Vec::new(),
            primary_recommendation: Recommendation {
                action: action.into(),
                rationale: "r".into(),
            },
            certainty_level: certainty,
            certainty_justification: "j".into(),
            data_completeness: DataCompleteness::Partial,
            red_flags: Vec::new(),
            follow_up_triggers: Vec::new(),
            disclaimer: "d".into(),
        }
    }

    #[test]
    fn classify_recognises_clear_actions() {
        use DecisionCategory as D;
        assert_eq!(D::classify("Resección anterior baja (cirugía)"), D::Surgery);
        assert_eq!(
            D::classify("Quimiorradioterapia neoadyuvante antes de la cirugía"),
            D::NeoadjuvantTherapy,
        );
        assert_eq!(
            D::classify("Mantener Watch & Wait con vigilancia estrecha"),
            D::WatchAndWait,
        );
        assert_eq!(
            D::classify("Seguimiento clínico y vigilancia"),
            D::Surveillance
        );
        assert_eq!(
            D::classify("Completar estadificación con RM y MMR/MSI"),
            D::FurtherStaging,
        );
        assert_eq!(
            D::classify("Cuidados paliativos / soporte sintomático"),
            D::Palliative
        );
        assert_eq!(D::classify("algo totalmente distinto"), D::Other);
    }

    #[test]
    fn concordance_levels() {
        use DecisionCategory as D;
        assert_eq!(
            Concordance::score(D::Surgery, D::Surgery),
            Concordance::Concordant
        );
        // adjacent → partial
        assert_eq!(
            Concordance::score(D::Surveillance, D::WatchAndWait),
            Concordance::Partial,
        );
        assert_eq!(
            Concordance::score(D::WatchAndWait, D::Surveillance),
            Concordance::Partial,
        );
        // unrelated → discordant
        assert_eq!(
            Concordance::score(D::Surgery, D::Palliative),
            Concordance::Discordant,
        );
    }

    #[test]
    fn report_stratifies_by_certainty() {
        let outcomes = vec![
            // high certainty, concordant
            CaseOutcome::score(
                "a".into(),
                DecisionCategory::Surgery,
                &verdict_with("cirugía: resección", CertaintyLevel::High),
            ),
            // low certainty, discordant
            CaseOutcome::score(
                "b".into(),
                DecisionCategory::Surgery,
                &verdict_with("cuidados paliativos", CertaintyLevel::Low),
            ),
        ];
        let report = ConcordanceReport::from_outcomes(outcomes);
        assert_eq!(report.overall.total(), 2);
        assert_eq!(report.by_certainty.high.strict_rate(), Some(1.0));
        assert_eq!(report.by_certainty.low.strict_rate(), Some(0.0));
        assert_eq!(report.by_certainty.medium.strict_rate(), None);
    }
}
