// Pure parsing helpers shared by the deliberation overlay and the
// failed-case views. No React — unit-testable in isolation.

import type { DeliberationPhase, Verdict } from "../../lib/ipc";

export const PHASE_ORDER: DeliberationPhase[] = [
  "briefing",
  "drafting",
  "redteam",
  "finalize",
];

/**
 * The deliberation pipeline persists failures as
 * `"deliberation phase {phase} failed: {provider error}"` — sometimes
 * with a leading `"provider error: "` prefix when the verdict pipeline
 * re-wraps the error. Extract the phase tag (briefing / drafting /
 * redteam / finalize) so the UI can render a localized header. Returns
 * `null` for legacy or non-deliberation errors so the caller falls back
 * to the generic "ejecución ha fallado" header.
 */
export function parseFailedPhase(error: string): DeliberationPhase | null {
  const match = error.match(
    /deliberation phase (briefing|drafting|redteam|finalize) failed:/,
  );
  return (match?.[1] as DeliberationPhase | undefined) ?? null;
}

/** Strip optional ```json fences from an LLM response so it can be
 *  fed straight to JSON.parse. Mirrors the Rust-side `strip_code_fences`
 *  in `crates/verdict/src/validation.rs`. */
export function stripCodeFences(s: string): string {
  const trimmed = s.trim();
  if (trimmed.startsWith("```json")) {
    return trimmed.slice("```json".length).trim().replace(/```$/, "").trim();
  }
  if (trimmed.startsWith("```")) {
    return trimmed.slice(3).trim().replace(/```$/, "").trim();
  }
  return trimmed;
}

/** Best-effort parse of a phase output into a `Verdict`. Returns `null`
 *  on any structural mismatch — caller falls back to a raw JSON block. */
export function tryParseVerdict(raw: string): Verdict | null {
  try {
    const parsed = JSON.parse(stripCodeFences(raw)) as Partial<Verdict>;
    if (
      parsed &&
      typeof parsed.case_summary === "string" &&
      parsed.primary_recommendation &&
      typeof parsed.primary_recommendation.action === "string" &&
      typeof parsed.primary_recommendation.rationale === "string" &&
      (parsed.certainty_level === "high" ||
        parsed.certainty_level === "medium" ||
        parsed.certainty_level === "low") &&
      Array.isArray(parsed.key_clinical_data) &&
      Array.isArray(parsed.red_flags) &&
      Array.isArray(parsed.follow_up_triggers) &&
      Array.isArray(parsed.applied_evidence)
    ) {
      return {
        case_summary: parsed.case_summary,
        key_clinical_data: parsed.key_clinical_data,
        applied_evidence: parsed.applied_evidence,
        primary_recommendation: parsed.primary_recommendation,
        certainty_level: parsed.certainty_level,
        certainty_justification: parsed.certainty_justification ?? "",
        data_completeness: parsed.data_completeness,
        red_flags: parsed.red_flags,
        follow_up_triggers: parsed.follow_up_triggers,
        disclaimer: parsed.disclaimer ?? "",
      };
    }
  } catch {
    /* fall through to null */
  }
  return null;
}
