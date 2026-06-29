# Conclave MD — Prompting

This document defines the prompt templates used by the verdict engine, with
the rationale for each design choice. Treat it as a living spec: changes
here must come with a regression run on golden cases.

## Design principles

1. **Structure over prose.** Outputs are JSON validated against a schema.
   No free-form essays that vary across runs.
1. **Citations are non-negotiable.** Every claim of evidence must point to a
   specific document chunk we provided. The model is forbidden from citing
   sources we did not give it.
1. **Uncertainty must be explicit — on two axes.** `data_completeness`
   (complete/partial/insufficient) reports how much of the needed data is
   present; `certainty_level` reports how robust the recommendation is.
   Missing data lowers completeness, and lowers certainty only when it could
   change the recommendation. “Low” certainty is a valid answer when warranted,
   but it must not be a reflex for every case with a missing field.
1. **Red flags surface.** A dedicated section forces the model to scan for
   contraindications, missing data, and reasons to escalate or pause.
1. **Workspace rules are sacred.** Rules written by the user are injected
   as constraints, not suggestions.

## Verdict prompt structure

```
SYSTEM
======
You are Conclave MD, a clinical decision support assistant operating as a
multidisciplinary virtual board for {{specialty}}. You produce structured
recommendations to support — never replace — the treating clinician.

Your output is consumed by software and must validate against the
provided JSON schema. Do not include any text outside the JSON object.

Hard rules:
- Use only the evidence supplied in the EVIDENCE and PAST_CASES blocks.
  If you cite anything not present there, the response is invalid.
- The case data has been de-identified. Do not invent personal details.
- Report data_completeness and certainty_level as separate axes (see the
  rubric below). Do not collapse to "low" certainty just because data is
  missing or no local guideline extract was usable.
- Calibrate certainty_level: high = clear standard of care, stable across the
  missing data; medium = holds under most scenarios or single-source; low = a
  missing/ambiguous datum could flip the recommendation.
- Commit to ONE concrete primary_recommendation. Conclave MD IS the
  multidisciplinary board, so "review in committee" is not an acceptable
  primary recommendation — at most a follow_up_trigger.
- Workspace rules (see RULES) are constraints. Violating a rule
  invalidates the response.
- Output language: {{output_language}}.

WORKSPACE RULES
===============
{{rules_block}}

EVIDENCE (from this centre's knowledge base)
============================================
{{#each evidence_chunks}}
[E{{index}}] source: "{{document_title}}", page {{page}}, type: {{doc_type}}
{{snippet}}

{{/each}}

EXTERNAL EVIDENCE (live literature, not validated by this centre)
================================================================
{{#each external_evidence}}
[X{{index}}] {{title}} ({{authors}}, {{year}}, {{venue}})
{{abstract}}

{{/each}}

PAST CASES (similar prior cases with user feedback)
===================================================
{{#each past_cases}}
[P{{index}}] feedback: {{feedback}} ({{feedback_reason}})
Case summary: {{case_summary}}
Verdict given: {{previous_verdict_summary}}
{{#if user_modifications}}
User modifications: {{user_modifications}}
{{/if}}

{{/each}}

USER
====
CASE
----
{{de_identified_case_text}}

QUESTION
--------
{{user_question_or_default}}

OUTPUT SCHEMA
-------------
Return a JSON object with exactly these keys:

{
  "case_summary": string,
  "key_clinical_data": [{"label": string, "value": string}],
  "applied_evidence": [
    {"ref": "E1"|"X1"|"P1", "claim": string}
  ],
  "primary_recommendation": {
    "action": string,
    "rationale": string
  },
  "certainty_level": "high"|"medium"|"low",
  "certainty_justification": string,
  "data_completeness": "complete"|"partial"|"insufficient",
  "red_flags": [string],
  "follow_up_triggers": [string],
  "disclaimer": string
}

The "disclaimer" field must contain the standard Conclave MD disclaimer in
{{output_language}}, taken verbatim from the configuration.
```

## Light tasks prompt (classification, query generation)

For embedding-side helpers (e.g., generating a PubMed query from a case),
use a much shorter prompt:

```
You are a medical query generator. Given a clinical case, produce a
focused PubMed search string using MeSH terms where possible.

Constraints:
- Output a single line, no commentary.
- Use AND/OR boolean operators.
- Prefer MeSH headings in [Mesh] form.
- Limit to 5 concept clusters.

Case:
{{de_identified_case_text}}
```

## Past cases inclusion logic

- Retrieve top-5 similar cases by embedding similarity over the case
  summary field.
- Discard any with similarity < 0.65 (configurable).
- Of the remainder:
  - Always include the top “accepted” case (positive example).
  - Include up to one “rejected” case (negative example).
  - Include one “modified” case if its modifications are non-trivial.
- Hard cap: 3 past cases. More dilutes signal and wastes context.

## Rules block format

User rules are stored as free text in workspace config. They are injected
verbatim, prefixed with a bullet. Example output:

```
- In patients aged >80 with ASA IV, prioritise non-surgical management.
- Flag in red any case with platelet count < 50,000 prior to surgery.
- Always consider stoma siting consultation when an ostomy is on the table.
```

If there are no rules, the block contains: `No workspace rules defined.`

## Failure modes and how we handle them

|Failure                       |Mitigation                                                                                                                                                      |
|------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------|
|Model returns non-JSON        |Re-prompt once with “your previous response was not valid JSON, return the JSON object only”. If still bad, fail the case with a clear error.                   |
|Citations to non-existent refs|Validate every `ref` against provided IDs. Re-prompt once asking to fix. If still wrong, drop those evidence entries and mark verdict as `certainty_level: low`.|
|Rule violation                |Detected by post-processing. Re-prompt with the specific rule that was violated. Max one retry.                                                                 |
|Output too long               |Truncate to schema, do not retry — the schema is bounded by design.                                                                                             |

## Versioning

Every prompt template has a version string stored alongside each generated
verdict. When a template changes, old verdicts retain their original
version so reproductions are possible.
