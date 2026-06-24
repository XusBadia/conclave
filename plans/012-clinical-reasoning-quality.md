# Plan 012: Clinical reasoning quality — RAG signal, confidence calibration, committed recommendations, colorectal priors, eval harness

> **Executor instructions**: Read the whole plan before starting. The five
> work-streams (W1–W5) are independently shippable in the wave order given
> under "Execution order". Run every verification command and confirm the
> expected result before moving on. Honor the STOP conditions — do not
> improvise. Every user-facing string is bilingual
> (`apps/desktop/src/locales/es.json` **and** `en.json`). Conventional
> commits (`fix(scope): …`, `feat(scope): …`). When done, update this plan's
> status row in `plans/README.md`.
>
> **Drift check (run first)**:
> `git log --oneline -8 crates/verdict/src/prompt.rs crates/verdict/src/pipeline.rs crates/verdict/src/schema.rs crates/verdict/src/deliberation.rs crates/rag/src/pipeline.rs crates/rag/src/extract/pdf.rs crates/cli/src/main.rs`
> The line numbers below were anchored at commit `cd4a991` and may have moved;
> re-grep the named **symbols**, not the line numbers. On a mismatch between a
> "Diagnosis" excerpt and the live code, treat it as a STOP condition.

## Status

- **Priority**: P1 (clinical validity — these four defects undermine the blinded CMT validation)
- **Effort**: XL overall (S–M per work-stream; ship in waves)
- **Risk**: MED–HIGH — W4 (colorectal priors) is clinically sensitive and **must** be clinician-authored/approved; W1/W2 change prompt + retrieval behavior and need empirical tuning against the guideline corpus
- **Depends on**: none (picks up the "Rules engine stated-but-undelivered" direction finding in `plans/README.md:74-78`)
- **Category**: product (clinical-reasoning-quality)
- **Planned at**: commit `cd4a991`, 2026-06-24

## Why this matters

We ran a **blinded validation** of Conclave against a real colorectal-cancer
multidisciplinary tumor board (CMT): 65 acts, each processed **without** its
resolution (`Pla d'actuació`), comparing Conclave's recommendation to the
board's real decision. Model: `gpt-5.5` via `codex-cli`. Four defects emerged,
and code reading confirmed a concrete root cause for each:

1. **Uncalibrated confidence** — `certainty_level = low` in 64/65 cases (1
   medium, 0 high). Confidence is effectively a constant and carries no signal.
2. **Unusable normative evidence** — the model repeatedly states the guideline
   extracts "only contain headers or copyright text," i.e. the guideline RAG
   injects boilerplate instead of recommendations, which **also depresses
   confidence**.
3. **Circular recommendation** — typical output is "do not close an indication;
   complete staging; review in the multidisciplinary committee." For cases that
   *already are* committee cases, "take it to committee" is a non-answer.
4. **A failed domain rule** — in a rectal-cancer Watch & Wait case with control
   MRI suggestive of regrowth and a **negative biopsy**, the board indicated
   surgery; Conclave recommended re-evaluating first, applying a generic
   heuristic ("confirm histology before acting") that is wrong in W&W (negative
   biopsy has a high false-negative rate and does not rule out regrowth).

The intended outcome: confidence that discriminates easy from ambiguous cases,
a guideline RAG that surfaces real recommendations, a concrete committed
primary action instead of deferral, colorectal safety rules that generic
heuristics cannot override, and a **reproducible harness** so we can re-run the
65-case concordance study and *measure* the improvement (especially
concordance stratified by confidence level).

## Root-cause map (confirmed against code at `cd4a991`)

| Defect | Confirmed root cause | Anchor |
|--------|----------------------|--------|
| #2 RAG noise | KB retrieval returns top-k (default 8) with **no relevance floor**; past cases use `past_cases_min_similarity = 0.65`. `VectorHit.distance` (L2) exists but is discarded. No boilerplate stripping at extraction; chunking not section-aware; no quality gate before injection. | `retrieve_evidence_with_vec` `crates/verdict/src/pipeline.rs:687`; `VerdictOptions` defaults `:90-96`; `render_evidence` `crates/verdict/src/prompt.rs:210`; `crates/rag/src/extract/pdf.rs`; `crates/rag/src/chunk.rs` |
| #1 confidence | Single rule "insufficient info → low"; **no** high/medium/low rubric; **no** data-completeness axis (the two are conflated); deliberation finalize only ever *lowers* certainty, never raises it. | `crates/verdict/src/prompt.rs:157-159`; `schema.rs:63-69`; `crates/verdict/src/deliberation.rs` red-team `~704` / finalize `~730` |
| #3 circular rec | Anti-hedging rule bans a *menu of alternatives* but not "review in committee" as the action itself; "multidisciplinary virtual board" persona reinforces deferral. | `crates/verdict/src/prompt.rs:143-145, 160-163`; `PROMPTING.md` |
| #4 domain rule | No domain-rule mechanism is wired: `VerdictOptions.rules_block` exists and is injected as `WORKSPACE RULES`, but defaults to `""` and nothing populates it. Skills are single-active, user-selected overlays — easy to "forget." | `rules_block` `pipeline.rs:56,91`; `prompt.rs:167`; `skills.rs` |

## Decisions taken (from review questions)

- **Confidence axis:** add a **structured `data_completeness` field** to the
  verdict output (not prompt-only), so the axis is first-class and measurable.
- **Domain rules:** **always-on specialty priors** loaded into the existing
  `rules_block` automatically by workspace specialty — cannot be forgotten or
  overridden by a generic skill.
- **Eval harness:** a **`conclave-cli eval` subcommand** over a local,
  gitignored case corpus (no PHI in the repo) **plus** 2–3 committed synthetic
  (non-PHI) golden fixtures that pin calibration behavior in `cargo test`.

---

## W1 — Guideline RAG signal quality  *(P1, M, risk MED)*

Fixes #2 and removes the artificial confidence depressor feeding #1.

**Diagnosis (confirmed):** `retrieve_evidence_with_vec`
(`crates/verdict/src/pipeline.rs:687-713`) calls `repository.search(query_vec,
top_k)` and discards `VectorHit.distance`. There is no relevance floor — unlike
`retrieve_past_cases` (`:715-732`) → `similar_past_cases` which applies
`past_cases_min_similarity = 0.65`. PDF extraction
(`crates/rag/src/extract/pdf.rs`) returns raw text with no boilerplate removal;
`chunk.rs` greedily packs sentences (target 700 / min 500 / overlap 100 tokens)
with zero section awareness; `render_evidence` (`prompt.rs:210`) renders every
chunk as `[E1..EN]` with no gate.

**Changes:**

1. **Relevance floor on KB retrieval** *(highest leverage, do first).*
   - Add `kb_min_relevance: f32` to `VerdictOptions` (mirror
     `past_cases_min_similarity`; `pipeline.rs:52-106`). Thread the hit
     distance out of `repository.search` (it is already on `VectorHit.distance`,
     `crates/rag/src/store/vector.rs`) into `retrieve_evidence_with_vec` and
     drop hits below the floor.
   - **Verify the embedding norm first:** confirm whether the fastembed model
     emits L2-normalized vectors (`crates/rag/src/embed*`). If normalized,
     convert LanceDB L2 distance `d` to cosine `cos = 1 - d²/2` and threshold on
     cosine for parity with the past-case path; if not, normalize at query/index
     time or threshold on raw distance. Document which path was taken.
   - Default **conservatively** and tune against the guideline corpus with W5
     (start ~0.30–0.45 cosine; the case→guideline query is not symmetric with
     case→case, so do not assume 0.65 transfers). Keep a **top-1 fallback** or a
     "floor never empties a non-empty corpus result silently" guard so a
     too-aggressive threshold can't regress to an empty `EVIDENCE` block.

2. **Boilerplate stripping at ingestion** *(makes the corpus clean for good).*
   - Add a conservative cleaner (new `crates/rag/src/extract/clean.rs` or a pass
     inside the extract step) that removes: lines recurring across many pages
     (running headers/footers), bare page numbers, copyright/ISBN/DOI/URL-only
     lines, and table-of-contents / index lines (dotted leaders `\.{3,}\s*\d+$`).
     Unit-test on real header/TOC vs. real recommendation samples; bias toward
     under-deletion.
   - **Operational note:** this only affects *newly ingested* documents. The
     already-ingested guideline corpus must be **re-ingested** for the effect to
     land (call this out in the PR and the acceptance run).

3. **"Usable-evidence" gate before injection** *(cheap belt-and-suspenders).*
   - In the evidence-assembly path feeding `render_evidence`, drop snippets that
     are mostly non-alphanumeric or look like a header/TOC fragment even after
     cleaning, so `[E*]` never carries pure boilerplate.

4. *(Stretch, optional)* Section-aware chunking for guideline structure
   (recommendation blocks vs. front-matter). Larger lift; defer unless W5 shows
   the floor + cleaner are insufficient.

**Privacy:** all local text processing — `crates/rag` makes **no** network
calls (invariant #1 holds).

**Tests:** unit tests for the cleaner (header/TOC/copyright dropped,
recommendation kept), and for the relevance floor (hits below threshold
filtered, ordering preserved, empty-guard works).

**Acceptance:** in the W5 re-run, the "extracts only contain headers/copyright"
complaint appears in **0** cases (was frequent); a clear majority of cases cite
≥1 real guideline recommendation in `applied_evidence` `[E*]`.

---

## W2 — Confidence calibration  *(P1, M, risk MED)*

Fixes #1. The headline metric problem.

**Diagnosis (confirmed):** the only certainty instruction is "If the supplied
information is insufficient for a confident answer, set certainty_level to low
and list the missing data in red_flags" (`prompt.rs:157-159`) — it **conflates**
missing data with low confidence. No high/medium/low rubric exists in any of the
four deliberation phases. Finalize is hard-wired to *lower* certainty on red-team
pushback ("Where the critique flagged certainty pushback, lower certainty_level",
`deliberation.rs:~730`) with no symmetric raise.

**Changes:**

1. **Add a structured `data_completeness` axis** (decision taken).
   - New enum `DataCompleteness { Complete, Partial, Insufficient }` in
     `schema.rs` (mirror `CertaintyLevel`, lowercase serde, `:63-69`). Add
     `#[serde(default)] pub data_completeness: DataCompleteness` to `Verdict`
     (`:8-33`) — `red_flags`/`follow_up_triggers` already use `#[serde(default)]`,
     so historical `output_json` blobs deserialize unchanged → **no SQL
     migration**. Add the field to the JSON schema mirror (`schema.rs:82+`) and
     the prompt's `OUTPUT SCHEMA` block (`prompt.rs:179-185`).
   - Update `validation::validate_verdict` if it enumerates required keys.
   - Render it in the frontend: `verdictParsing.ts`, `VerdictRenderer.tsx`,
     `CaseVerdictPDF.tsx`, the `ipc.ts` type, and **bilingual** locale keys under
     the existing `verdict.*` block (`en.json:423-436` + `es.json`).

2. **Rewrite the certainty rule + add an explicit rubric** (`prompt.rs:157-163`).
   - Decouple the axes: certainty reflects **how robust the recommendation is to
     the residual uncertainty**, not whether every field is populated. Missing
     data feeds `data_completeness` and `red_flags`, and lowers certainty **only
     when it could plausibly flip the recommendation.**
   - Add a high/medium/low rubric with anchored examples:
     - **high** — recommendation is the clear standard of care and stable across
       plausible values of the missing data (e.g. *pT2N0 → surveillance*; a clear
       palliative situation).
     - **medium** — recommendation holds under most but not all plausible
       scenarios, or rests on indirect/single-source evidence.
     - **low** — a missing/ambiguous datum could realistically change the primary
       recommendation.
   - Explicitly: **absence of local guideline extracts does not by itself cap
     certainty** when standard-of-care is clear (directly counters the #2→#1
     coupling).

3. **Make finalize symmetric** (`deliberation.rs:~730`).
   - Change "lower certainty_level" to "**adjust** certainty_level (raise or
     lower) to match the rubric." Add a red-team check (`~704`): "is certainty
     **too low** for a clear standard-of-care answer?" so the critique can push
     confidence **up**, not only down.

4. **Bump `VERDICT_PROMPT_VERSION`** `verdict_v2` → `verdict_v3` (`prompt.rs:15`)
   and update the empty-inputs / schema tests + `PROMPTING.md`.

**Acceptance:** in the W5 re-run the certainty distribution spreads (not ≥95%
low); concordance is **monotone in certainty** (concordance@high ≥ @medium ≥
@low). The committed synthetic goldens (W5) pin: a pT2N0-style case → not `low`
+ surveillance; a genuinely ambiguous case → `low`.

---

## W3 — Committed, non-circular primary recommendation  *(P1, S, risk LOW)*

Fixes #3. Prompt-only, cheapest clinical win.

**Diagnosis (confirmed):** the "Commit to a single primary_recommendation … do
not hedge with a menu of alternative options" rule (`prompt.rs:160-163`) bans an
alternatives menu but not "review in committee" as the *action*; the
"multidisciplinary virtual board" persona (`:143-145`) actively reinforces
deferral.

**Changes (`prompt.rs`, `PROMPTING.md`):**

1. Add a hard rule: `primary_recommendation.action` must be a **concrete
   clinical action** (e.g. "complete pelvic MRI + MMR/MSI status, then proceed to
   TME if restaging confirms…"), **never** "review in the multidisciplinary
   committee" — *Conclave is the board*. Committee review may appear only as a
   `follow_up_trigger` or a stated condition, never as the primary action.
2. Reframe "commit" into: **one primary action + the assumptions it rests on +
   what finding would change it** (folded into `rationale` / `follow_up_triggers`).
   This satisfies both "commit" and honest contingency without circular hedging.
3. Update `PROMPTING.md` to match and fix the stale `alternatives` field it still
   documents (removed from `schema.rs`).

**Acceptance:** in the W5 re-run, "review in committee" as the primary action
drops to ~0; primary actions are concrete and gradeable against the board's
real decision.

---

## W4 — Always-on colorectal domain priors  *(DROPPED by maintainer decision)*

> **Status: DROPPED.** Hardcoding clinical facts in the binary is stale-prone
> and redundant: the model already knows standard colorectal oncology, and #4
> is most likely a *symptom* of #1/#2/#3 (noisy RAG + over-cautious prompting),
> which W1/W2/W3 address. We removed the priors and will check whether #4
> resolves on revalidation. If a domain rule is still needed, its correct home
> is **user-editable workspace rules** (clinician-owned, non-stale) via the
> still-empty `rules_block` — not baked-in content. The text below is retained
> for context only.

Fixes #4. Decision taken: **always-on specialty priors via `rules_block`.**

**Diagnosis (confirmed):** `rules_block` is injected as `WORKSPACE RULES`
(`prompt.rs:167`) but defaults to `""` (`pipeline.rs:91`) and nothing populates
it (`plans/README.md:74-78`). So no domain rule can currently outrank a generic
model heuristic.

**Changes:**

1. **Specialty-prior loader.** Add a small store of curated, versioned priors
   keyed by specialty (bundled markdown under e.g.
   `crates/verdict/src/priors/colorectal.md`, loaded via a `load_specialty_priors(specialty)`
   helper). Prepend the matched priors into `rules_block` **at the two
   construction sites** where the workspace specialty is already in scope:
   `apps/desktop/src-tauri/src/commands.rs:~2928` and
   `crates/cli/src/commands/case.rs`. Because they ride in `WORKSPACE RULES`,
   they are already framed as **hard constraints** ("Violating a rule invalidates
   the response", `prompt.rs:164-165`) and cannot be silently dropped.
2. **Seed colorectal priors (drafts — clinician must review/approve before
   shipping; bilingual es/en):**
   1. **Rectal W&W / regrowth:** a negative biopsy does **not** rule out local
      regrowth; regrowth is a clinical-radiological diagnosis (endoscopy + MRI +
      DRE). Do not defer salvage surgery solely because a biopsy is negative when
      imaging/endoscopy suggest regrowth. *(The illustrative failure case.)*
   2. **Locally advanced rectal cancer (cT3–4 or cN+):** neoadjuvant therapy
      (chemoRT / TNT) precedes TME; do not recommend upfront surgery.
   3. **All CRC:** MMR/MSI status should be determined — it drives Lynch
      screening, prognosis, and dMMR-metastatic immunotherapy eligibility.
   4. **Stage II colon adjuvant:** the chemo decision hinges on high-risk
      features (T4, obstruction/perforation, <12 nodes, LVI, poor differentiation)
      **and** MMR status (dMMR stage II generally does not benefit from 5-FU
      monotherapy).
   5. **Complete clinical response after neoadjuvant (rectal):** organ-preserving
      W&W is a legitimate option for cCR; surgery is not the only acceptable path,
      but missing/contradictory restaging warrants *completion of staging*, not a
      default to surgery.
   - Each prior carries a one-line source/guideline citation and is written to be
     **generalizable**, not overfit to the single W&W case.

**Risk / STOP:** these are clinically sensitive. **Do not ship any prior without
the maintainer-clinician's review and sign-off** — the drafts above are a
starting point, not approved content. Keep the ruleset conservative and versioned.

**Acceptance:** the illustrative W&W biopsy-negative-regrowth case flips to
"proceed toward surgery" and matches the board; **no regression** on existing
golden fixtures or the rest of the 65-case set.

---

## W5 — Reproducible evaluation harness  *(P1, M, risk LOW)*

The yardstick. Decision taken: **CLI `eval` subcommand + committed synthetic
goldens.** Build the skeleton **first** so W1–W4 are each measured.

**Diagnosis (confirmed):** no batch-evaluation/concordance harness exists. Golden
tests (`pipeline.rs` `mod golden`, `~1154`) are behavioral/invariant (PII
masking, structure, persistence) over fixtures `{name, text, question, pii}`;
the CLI `export` command only serializes cases to JSON.

**Changes:**

1. **`conclave-cli eval` subcommand.** New `Command::Eval` variant
   (`crates/cli/src/main.rs:77`, dispatch `:134-146`) + `crates/cli/src/commands/eval.rs`,
   following the existing subcommand pattern (e.g. `case.rs`).
   - Input: a **local, gitignored** directory of case files + an
     expected-decisions file (CSV/JSON) mapping each case to the board's real
     decision and a 3-level concordance taxonomy (concordant / partially
     concordant / discordant — or a clinical-category map).
   - Reuse `VerdictPipeline` to run each case; collect primary action, certainty,
     `data_completeness`, and an evidence-usable flag.
   - Output: a report with overall concordance **and concordance stratified by
     certainty level** (the calibration metric), plus the evidence-usability rate.
   - **Privacy:** gitignore the corpus dir, reuse `deident`, never persist raw,
     **no PHI committed**. Concordance scoring (string/category match) is
     deterministic and unit-testable.
2. **Committed synthetic golden fixtures (no PHI).** Add 2–3 hand-written cases
   that pin calibration: a pT2N0-style case must yield non-`low` + surveillance;
   a deliberately ambiguous case must yield `low`. Wire them into the existing
   golden harness so `cargo test` (and the pre-commit hook) guards against
   calibration regressions.

**Acceptance:** a single command reproduces the 65-case concordance study and
prints calibration-stratified concordance; a before/after run demonstrates the
improvement from W1–W4. The synthetic goldens fail if calibration regresses.

---

## Execution order (waves)

1. **Wave A — foundation + yardstick (low risk, high leverage):**
   W5 skeleton (so everything is measured) → W1 relevance floor + cleaner →
   W3 recommendation prompt. Re-run the 65 cases to establish the post-A baseline.
2. **Wave B — the headline:** W2 calibration (field + rubric + symmetric
   finalize). Re-run; check concordance-by-certainty monotonicity.
3. **Wave C — clinician-gated + lock-in:** W4 colorectal priors (after clinician
   sign-off) → lock the calibration goldens (W5 part 2). Final 65-case re-run for
   the before/after report.

Soft dependency: W2 reads cleaner after W1 (no noisy evidence dragging confidence
down). W4 needs the small specialty-prior loader; otherwise independent.

## Global verification

- **Gate:** `./scripts/verify.sh` green (fmt; clippy pedantic+nursery
  `-D warnings`; `cargo test --workspace`; `pnpm --dir apps/desktop build`).
- **New Rust tests:** relevance-floor filter + empty-guard; boilerplate cleaner;
  presence of the calibration rubric / `data_completeness` in the rendered prompt
  and schema; specialty-prior injection into `rules_block`; concordance scorer;
  the new synthetic calibration goldens.
- **Frontend:** `pnpm --dir apps/desktop test` for the `data_completeness`
  rendering; both `es.json` and `en.json` updated (no missing-key warnings).
- **End-to-end:** run `conclave-cli eval` over the local 65-case corpus before
  and after each wave; the deliverable is the calibration-stratified concordance
  comparison.

## Risks & mitigations

- **W4 clinical correctness** → clinician authors/approves every prior; versioned;
  conservative; cited; generalizable (not overfit to one case). **Hard STOP** on
  shipping unreviewed rules.
- **W1 floor too aggressive empties evidence** → conservative default, top-1/empty
  guard, tune via W5 on the real corpus.
- **W1 cleaner over-deletes** → conservative heuristics, unit-tested on real
  header/TOC vs. recommendation; ingestion-only; re-ingest required (note in PR).
- **L2↔cosine conversion** → verify embeddings are normalized before converting;
  otherwise normalize or threshold raw distance.
- **`data_completeness` breaks old verdicts** → must use `#[serde(default)]`;
  STOP if any stored verdict fails to deserialize.
- **Prompt-version bump** invalidates byte-repro of pre-`v3` verdicts → expected
  and versioned; not a regression.

## STOP conditions

- Relevance floor empties the evidence block for a majority of cases at the
  chosen threshold → stop, re-tune; do not ship an empty-evidence regression.
- Any colorectal prior unreviewed by the clinician → do not ship W4.
- `data_completeness` causes historical-verdict deserialization failures → stop,
  fix the `serde(default)` path.

## Scope

**In scope:** KB relevance floor + boilerplate cleaner + evidence gate (W1);
`data_completeness` field + certainty rubric + symmetric finalize (W2);
non-circular committed-recommendation prompt rules (W3); always-on colorectal
priors via `rules_block` (W4); `conclave-cli eval` concordance harness +
synthetic calibration goldens (W5); bilingual locale keys; `PROMPTING.md` update;
`VERDICT_PROMPT_VERSION` bump.

**Out of scope:** full section-aware chunking (W1 stretch only); a general
workspace-rules CRUD UI (this plan only auto-loads bundled specialty priors —
the broader "rules engine" direction finding stays open); committing any real
patient corpus; changing the deliberation phase structure beyond the
certainty-adjustment symmetry; non-colorectal specialty priors.
