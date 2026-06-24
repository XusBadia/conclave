# Concordance evaluation harness

`conclave-cli eval` re-runs a batch of cases through the verdict pipeline and
scores **3-level concordance** (concordant / partial / discordant) against a
known committee decision, **stratified by certainty** — so each pipeline change
(plan 012 and beyond) can be measured, especially whether confidence is
calibrated (high-certainty cases should concord more than low-certainty ones).

## Privacy

**Never commit real patient data.** Keep a real corpus under `eval/corpus/`
(git-ignored). Only `eval/synthetic.json` (made-up vignettes, no PHI) is
committed. Each case is de-identified before any prompt, exactly like
`case new`, and raw text is purged after the run unless `--retain-raw-text`.

## Manifest format

A JSON array. Each case has an `id`, an `expected_category`, a `question`
(optional), and the case text via either `text` (inline) or `text_file` (a path
relative to the manifest):

```json
[
  { "id": "c1", "text": "…", "expected_category": "surveillance" },
  { "id": "c2", "text_file": "corpus/c2.txt", "expected_category": "surgery" }
]
```

`expected_category` is one of: `surgery`, `neoadjuvant_therapy`,
`adjuvant_therapy`, `systemic_therapy`, `surveillance`, `watch_and_wait`,
`further_staging`, `palliative`, `other`.

## Running

```sh
# Smoke-test the harness on the committed synthetic cases:
conclave-cli eval --manifest eval/synthetic.json --provider <id>

# Reproduce the real concordance study (local corpus, JSON report out):
conclave-cli eval --manifest eval/corpus/cmt65.json --provider codex-cli \
  --output eval/corpus/report.json
```

Cases are run independently by default (`--past-cases-k 0`) so memory does not
leak between cases. Tune the KB relevance floor with `--kb-min-relevance`
(0 disables it; see plan 012 W1).

## Caveat

The predicted category is assigned by a conservative keyword classifier
(`conclave_verdict::DecisionCategory::classify`); it is an aid, not an oracle.
The per-case table and the JSON report always include the raw recommendation so
a clinician can override a misclassification before trusting the aggregate.
