# Plan 006: Truth-up stale core docs and add a root CLAUDE.md for agent sessions

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 4ecaaea..HEAD -- ARCHITECTURE.md RUNNING.md README.md CLAUDE.md`
> If any of these changed since this plan was written, re-verify each stale
> claim below against the live repo before rewriting it.

## Status

- **Priority**: P2
- **Effort**: S–M
- **Risk**: LOW (documentation only; the risk is writing NEW falsehoods — every claim you write must be re-verified against code)
- **Depends on**: none (if Plan 004 has landed, mention its helper in CLAUDE.md; check `plans/README.md` status)
- **Category**: docs / dx
- **Planned at**: commit `4ecaaea`, 2026-06-10

## Why this matters

This repo is developed primarily through AI agent sessions (see README's
Claude Code workflow and `.claude/napkin.md`). Its two foundational docs
contain claims that are now false, and stale docs actively misdirect both
human contributors and the agents executing plans:

- **ARCHITECTURE.md** describes a GitHub Actions CI matrix (3 OSes,
  fmt/clippy/test on every push) that does not exist — the only workflow is
  `release.yml`, fired on version tags. A contributor who skips the local
  hook setup believing "CI will catch it" pushes unverified code.
- **ARCHITECTURE.md** shows a workspace tree with a `docs/` directory and
  `test-fixtures/` that don't exist, and omits `crates/verdict` — the
  product's core crate — from the layout.
- **RUNNING.md** says Phase 5 (learning loop) and Phase 6 (online evidence)
  are "deferred / not built". Both are now substantially built and wired
  (verified at commit 4ecaaea — details below). A user reading the docs
  doesn't know two flagship capabilities exist; a developer plans work that's
  already done.
- There is **no root `CLAUDE.md`**, so every agent session re-discovers the
  build commands, privacy invariants, and the repo's hard-won gotchas
  (`.claude/napkin.md` holds them but is loaded only by sessions that know to
  read it).

## Current state

Verified facts to write the corrections from (re-verify each with the given
command before relying on it):

| Claim in docs | Reality | Re-verify with |
|---|---|---|
| ARCHITECTURE.md §CI (lines ~183–194): "GitHub Actions matrix: ubuntu/macos/windows … each job: fmt, clippy, test, build" | Only `.github/workflows/release.yml`, triggered by `v*` tags + manual dispatch; day-to-day verification is `./scripts/verify.sh` via the versioned pre-commit hook (`git config core.hooksPath .githooks`, documented in README §Local verification) | `ls .github/workflows/` |
| ARCHITECTURE.md workspace tree (lines ~21–47): shows `.github/workflows/ci.yml`, `docs/` dir, `test-fixtures/`; omits `crates/verdict` and `apps/web` | No `ci.yml`, no `docs/` (docs live at repo root), no `test-fixtures/`; crates are core, providers, rag, deident, **verdict**, evidence, cli; apps are desktop AND web (Next.js marketing site) | `ls crates/ apps/ docs 2>&1` |
| RUNNING.md §"What's deferred": "Phase 5 — Learning loop (partial) … case-memory embedding + past-cases retrieval in the verdict prompt is not yet hooked up" | The pipeline upserts case memory after every verdict (`crates/verdict/src/pipeline.rs:634` `upsert_case_memory(...)`) and the prompt template renders a past-cases block (`crates/verdict/src/prompt.rs:112` `render_past_cases`, used in the template at line ~172) | `grep -n "upsert_case_memory" crates/verdict/src/pipeline.rs; grep -n "render_past_cases" crates/verdict/src/prompt.rs` |
| RUNNING.md §"What's deferred": "Phase 6 — Online evidence. PubMed + Europe PMC adapters are not built." | `crates/evidence/src/pubmed.rs` and `europe_pmc.rs` exist with tests; the desktop exposes `use_online_evidence` on run requests and fetches via `fetch_external_evidence_for_case` (`apps/desktop/src-tauri/src/commands.rs:1512`); the CLI has an `Evidence` subcommand (`crates/cli/src/main.rs`, Command enum) | `ls crates/evidence/src/; grep -n "fetch_external_evidence_for_case" apps/desktop/src-tauri/src/commands.rs` |

Also relevant:

- README.md §Local verification correctly describes the hook — keep it; the
  ARCHITECTURE.md fix should point to it rather than duplicating.
- `.claude/napkin.md` — the gotcha log (Tauri mutex-across-await, SQLite
  migration pattern, Pointer-events drag workaround, CLI provider probing,
  batch panic forensics). CLAUDE.md should POINT to it, not duplicate it
  (the napkin churns; CLAUDE.md should stay stable).
- ARCHITECTURE.md §Privacy invariants (lines ~196–206) — accurate and
  load-bearing; CLAUDE.md should reference (not restate) them.
- ARCHITECTURE.md says `case new` etc. under `cli` §; CLI subcommands ALSO
  include `rules add|list|remove` in that doc — **no `rules` subcommand
  exists** (`grep -n "Rules" crates/cli/src/main.rs` → nothing). Note this in
  the rewrite (either mark as planned or remove; see Step 2.3).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Re-verify claims | (per-claim commands in the table above) | matches "Reality" column |
| Repo gate (docs don't break it, but run before committing) | `./scripts/verify.sh` | "All local checks passed" |

## Scope

**In scope**:

- `ARCHITECTURE.md` (CI section, workspace tree, CLI subcommand list)
- `RUNNING.md` ("What's deferred" section; data-path claims ONLY if you
  verify them wrong)
- `CLAUDE.md` (create at repo root)

**Out of scope** (do NOT touch):

- `PLAN.md`, `PROMPTING.md`, `DISCLAIMER.md`, `CONTRIBUTING.md`, `README.md`
  — not audited as stale (README verified accurate on the hook setup).
- `.claude/napkin.md` — agent-maintained; leave it.
- Any source code. This plan changes zero behavior.
- Marketing site content (`apps/web`).

## Git workflow

- Branch: `advisor/006-docs-truth-up`
- Commit style: `docs: align ARCHITECTURE/RUNNING with reality + add CLAUDE.md`.
- Pre-commit hook runs the full gate even for docs commits (~3–5 min); that's
  expected.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Fix ARCHITECTURE.md

1. Replace the §CI section: describe the actual model — local verification
  via `./scripts/verify.sh` (4 steps, listed), enforced by the versioned
  pre-commit hook (`git config core.hooksPath .githooks`, one-time per
  clone), plus `release.yml` building macOS-arm64/Linux/Windows bundles on
  `v*` tags. State explicitly: **no CI runs on push or PR.**
2. Fix the workspace tree: remove `.github/workflows/ci.yml`, `docs/`,
  `test-fixtures/`; add `crates/verdict/` ("multi-phase verdict engine:
  deliberation, persistence, skills, workflows"), `apps/web/` ("Next.js
  marketing site"), `prompts/`, `scripts/`, `plans/`.
3. CLI section: annotate `rules add|list|remove` as **not yet implemented**
  (it is referenced by PLAN.md as future work) rather than silently deleting
  it — the asymmetry is recorded as a direction finding in plans/README.md.

**Verify**: `grep -n "ci.yml\|ubuntu-latest" ARCHITECTURE.md` → no matches.
`grep -n "verdict" ARCHITECTURE.md` → at least one hit in the tree + crate sections.

### Step 2: Fix RUNNING.md

Rewrite §"What's deferred" to reflect verified reality:

1. Phase 5: case-memory upsert + past-cases prompt block are implemented;
   say what IS still open, if anything — before writing, check whether
   retrieved past cases flow in the **deliberated** path too:
   `grep -n "past_cases\|retrieve_past_cases\|similar_past_cases" apps/desktop/src-tauri/src/commands.rs crates/verdict/src/deliberation.rs | head`.
   Describe exactly what you find (e.g. "wired in quick + deliberated modes"
   or "wired in quick mode; deliberated mode does not yet inject past
   cases"). Do not guess.
2. Phase 6: adapters exist (PubMed + Europe PMC, cached in SQLite); surfaced
   via the `use_online_evidence` toggle on runs and the CLI `evidence`
   subcommand. Same rule: verify the UI toggle exists before claiming it
   (`grep -n "use_online_evidence" apps/desktop/src/routes/Cases.tsx | head -3`).
3. If after verification something genuinely remains deferred, keep a
   truthful, smaller "still open" list.

**Verify**: `grep -n "not yet hooked up\|adapters are not built" RUNNING.md` → no matches.

### Step 3: Create CLAUDE.md

Create `CLAUDE.md` at the repo root, ~80–140 lines, with exactly these
sections (content from verified reality, written tersely — it is loaded into
every agent session, so brevity is a feature):

1. **What this is** — 2 lines: local-first clinical decision-support;
   Rust workspace + Tauri 2 desktop + Next.js marketing site.
2. **Build / verify commands** — the table: `./scripts/verify.sh` (the gate),
   its four steps individually for targeted runs, `pnpm --dir apps/desktop build`,
   `pnpm tauri dev` caveat (kill any installed `/Applications/Conclave.app`
   first — same bundle id steals focus/screenshots), and the one-time
   `git config core.hooksPath .githooks`.
3. **Workspace map** — one line per crate/app (core, providers, rag,
   deident, verdict, evidence, cli, apps/desktop incl. `src-tauri`
   commands.rs as the IPC surface, apps/web).
4. **Privacy invariants** — reference ARCHITECTURE.md §Privacy invariants as
   normative; list the four headline rules in one line each (no network in
   core/rag/deident; de-identify before any LLM prompt; secrets in
   keychain/0600 files only; no telemetry).
5. **Hard-won gotchas** — pointer to `.claude/napkin.md` as the living log,
   plus the four evergreen rules inline:
   - never do byte arithmetic on UTF-8 without `is_char_boundary` walking
     (a panic here froze production batches);
   - never hold a `std::sync::MutexGuard` across `.await` in Tauri commands;
   - credential-less providers must be in the single bypass list in
     commands.rs (search `KEYCHAIN_LESS_PROVIDERS` if Plan 004 landed, else
     `grep "no API key for"`) — a miss is a silent no-op in batch mode;
   - UI icons are `@tabler/icons-react`, never emoji.
6. **Conventions** — clippy pedantic+nursery at `-D warnings`; `thiserror`
   in lib crates / `anyhow` only in `cli`; `tracing` not `println!`;
   bilingual i18n (every user-facing string needs `es` + `en`);
   conventional-commit messages (`fix(scope): ...`).
7. **Plans** — one line: `plans/` holds advisor-written implementation plans
   with their own index and statuses.

**Verify**: `wc -l CLAUDE.md` → between 60 and 160.
`grep -n "verify.sh\|napkin\|is_char_boundary\|tabler" CLAUDE.md` → all four hit.

### Step 4: Full gate + claim audit

Re-run every "Re-verify with" command from the Current state table and
confirm the rewritten docs match. Then:

**Verify**: `./scripts/verify.sh` → "✓ All local checks passed".

## Test plan

No automated tests (docs). The "test" is Step 4's claim audit: every factual
sentence you wrote must be backed by a command you ran in this session.

## Done criteria

ALL must hold:

- [ ] `grep -n "ubuntu-latest\|ci.yml" ARCHITECTURE.md` → empty
- [ ] `grep -n "docs/ARCHITECTURE\|test-fixtures" ARCHITECTURE.md` → empty (or explicitly marked "(planned)")
- [ ] `grep -n "not yet hooked up\|adapters are not built" RUNNING.md` → empty
- [ ] `CLAUDE.md` exists at repo root with the seven sections
- [ ] `./scripts/verify.sh` exits 0
- [ ] Only the three docs files modified/created (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- A "Reality" claim in the table fails its re-verification command (the code
  moved; the plan's facts are stale — report what you found instead).
- You find the Phase 5/6 wiring is partial in a way that takes more than a
  paragraph to describe honestly — report the findings; don't paper over
  them with vague wording.
- You're tempted to edit a file in the out-of-scope list (e.g. PLAN.md
  contradictions) — note the contradiction in your report instead.

## Maintenance notes

- CLAUDE.md must stay short and stable; volatile lessons go to
  `.claude/napkin.md`. Reviewers should reject CLAUDE.md PRs that paste
  napkin content wholesale.
- When CI-on-push is ever added, ARCHITECTURE.md §CI and CLAUDE.md §2 both
  change — grep for "verify.sh" to find every claim site.
- Deferred: a `docs/` directory consolidation (the root-level layout is fine;
  reshuffling files breaks deep links for no functional gain).
