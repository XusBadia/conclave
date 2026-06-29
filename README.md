# Conclave MD Bootstrap

Everything you need to hand to Claude Code to build Conclave MD end-to-end.

## What’s inside

```
conclave-bootstrap/
├── README.md                  ← you are here
├── docs/                      ← copy these into your repo at /docs
│   ├── README.md
│   ├── ARCHITECTURE.md
│   ├── PLAN.md
│   ├── PROMPTING.md
│   ├── DISCLAIMER.md
│   └── CONTRIBUTING.md
└── prompts/                   ← paste one at a time to Claude Code
    ├── phase-0-foundations.md
    ├── phase-1-knowledge-base.md
    ├── phase-2-providers.md
    ├── phase-3-deidentification.md
    ├── phase-4-verdict-engine.md
    ├── phase-5-learning-loop.md
    ├── phase-6-online-evidence.md
    └── phase-7-8-ui-and-distribution.md
```

## How to use

### Step 1 — Seed the repo with the docs

The repo exists already. From iPhone, you have two easy ways to add
these files:

**Option A — Tell Claude Code to do it.**

After your current Phase 0 session ends (or in a fresh one), paste:

> “I’m attaching seven Markdown files (the ones in `docs/`). Place them
> in the `docs/` directory at the repo root, exactly as named, and
> commit them with message `docs: add architecture, plan and guidelines`. Push to main.”

Then upload the seven files from `docs/` in the chat with Claude Code.

**Option B — Upload via GitHub mobile.**

In GitHub mobile, open the repo, tap `+` → Upload files → select the
seven files from `docs/`. Commit directly to main. Faster but uses
GitHub’s basic editor.

### Step 2 — Phase 0

If you’ve already kicked off Phase 0 with the earlier prompt, perfect
— let it finish. The detailed `phase-0-foundations.md` here is the
same spirit, fleshed out a bit more; if Phase 0 is still in progress
you can ignore this file. If it’s not started yet, use this version.

### Step 3 — Subsequent phases

For each subsequent phase:

1. Wait until the previous phase is fully green locally (`./scripts/verify.sh`).
1. Open a fresh Claude Code session in the repo.
1. Paste the corresponding `phase-N-*.md` file as the first message.
1. Claude Code will plan, post the plan, then implement, then commit
   and push.
1. Come back to the Conclave MD chat with me (Claude) if you want to
   refine anything before the next phase.

### Step 4 — Phase 4 needs a checkpoint

Phase 4 is the verdict engine and it’s the most product-defining part.
The prompt instructs Claude Code to **stop after planning and wait for
your review** before coding. When that happens, paste the proposed
plan back into our chat and we iterate together. Don’t let it implement
on autopilot — that’s the 80%-of-the-product moment.

## Tips for working with Claude Code on iPhone

- Push directly to `main` while solo; PR discipline later.
- If a session is long, summarise progress as a comment in the
  tracking issue before closing — restart Claude Code reads the issue
  faster than the full chat history.
- If the pre-commit hook fails, screenshot the failure and paste the
  error into a new Claude Code session: “fix the failure shown below,
  then commit and push”.
- If Claude Code starts making decisions you didn’t approve (changing
  stack, adding deps, restructuring crates), interrupt and point it
  at `docs/ARCHITECTURE.md`.

## Local verification (Git hook)

Conclave MD runs **no CI on push or PR** — the project is local-first and so
is the verification loop. After cloning, activate the versioned Git hook
once per clone:

```sh
git config core.hooksPath .githooks
```

Every `git commit` will then run, before creating the commit:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked --quiet`
4. `pnpm --dir apps/desktop build`

On a warm Cargo cache it takes ~3-5 minutes. If a step fails, the commit
is aborted. Bypass for an emergency commit with `git commit --no-verify`,
but don’t make it a habit.

You can also run the same checks on demand without committing:

```sh
./scripts/verify.sh
```

GitHub Actions only fires when you push a release tag (`vX.Y.Z`),
producing release binaries for macOS, Linux and Windows. Day-to-day
pushes to `main` don’t burn Actions minutes.

## Decisions already made (don’t relitigate)

- **Stack**: Rust core + Tauri 2 UI. Not Electron, not pure Swift, not
  pure web.
- **Embeddings**: `multilingual-e5-small` via `fastembed-rs`.
- **Storage**: SQLite + LanceDB, local under user data dir.
- **Providers**: API-key first, OAuth-subscription optional, local
  (Ollama + Apple Intelligence) supported.
- **Privacy**: mandatory de-identification before any LLM call.
- **License**: MIT.
- **Phase order is fixed**: don’t skip ahead.

## What’s deliberately not specified

- Exact dependency versions: let Claude Code pick recent stable, then
  pin via `Cargo.lock`.
- UI design tokens: defined later in Phase 7 with the Mac in hand.
- Specific NER model choice: locked in during Phase 3.
- OAuth providers (Phase 2.5): skipped unless we hit a real need.

Good luck. Ship it.
