# CLAUDE.md

## What this is

Conclave: a local-first clinical decision-support desktop app (a "virtual
clinical committee" over the clinician's own protocols). Rust workspace +
Tauri 2 desktop app (React 18/TS frontend) + Next.js marketing site. It
handles real patient data — read the privacy invariants below before
touching anything.

## Build / verify

| What | Command |
|------|---------|
| THE gate (pre-commit hook runs this) | `./scripts/verify.sh` |
| Format | `cargo fmt --all --check` |
| Lint (pedantic+nursery, deny warnings) | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Rust tests | `cargo test --workspace --locked --quiet` |
| Frontend typecheck+build | `pnpm --dir apps/desktop build` |
| Frontend tests (vitest) | `pnpm --dir apps/desktop test` |
| Hook setup (once per clone) | `git config core.hooksPath .githooks` |

No CI on push — the hook is the only gate; release bundles build on `v*`
tags. `pnpm tauri dev` caveat: kill any installed `/Applications/Conclave.app`
first — both share the bundle id and the installed one steals
focus/screenshots.

## Workspace map

- `crates/core` — shared types, config (`PrivacyConfig` etc.), paths, logging.
- `crates/providers` — `LlmProvider` trait + impls (Anthropic/OpenAI/
  OpenRouter APIs, OAuth pair, Ollama, Apple Intelligence, `claude-cli` /
  `codex-cli` subprocess proxies), keychain `secrets`, OAuth flows.
- `crates/rag` — ingestion (PDF/DOCX/HTML/OCR), chunking, fastembed
  embeddings, LanceDB + SQLite.
- `crates/deident` — PII masking. Privacy-critical; property-tested.
- `crates/verdict` — the product core: quick pipeline + 4-phase
  deliberation, SQLite persistence (cases/verdicts/audit_runs), skills,
  golden-case tests in `pipeline.rs` + `tests/fixtures/`.
- `crates/evidence` — PubMed / Europe PMC + cache.
- `crates/cli` — `conclave-cli`.
- `apps/desktop/src-tauri/src/commands.rs` — the IPC surface (~4k lines;
  every UI capability lands here).
- `apps/desktop/src/routes/` — pages; the Cases route is decomposed under
  `routes/cases/` (helpers, banners, dialogs, overlay, `useBatchProgress`).
- `apps/web` — marketing site (independent).

## Privacy invariants (normative: ARCHITECTURE.md §Privacy invariants)

1. No network calls from `core`, `rag`, `deident`.
2. De-identify before ANY prompt with patient text; persist masked, never raw.
3. Secrets in the OS keychain; OAuth token files are created 0600
   (`providers::oauth_flow::write_secret_file` — use it for any new
   credential write).
4. No telemetry. The webview runs under a strict CSP (`tauri.conf.json`).

## Hard-won gotchas (living log: `.claude/napkin.md`)

- NEVER do byte arithmetic on UTF-8 without `is_char_boundary` walking —
  a deident slice panic froze production batches once; proptests in
  `crates/deident` now pin the class.
- NEVER hold a `std::sync::MutexGuard` across `.await` in Tauri commands
  (not Send; cryptic `invoke_handler!` errors).
- Credential-less providers MUST be in `KEYCHAIN_LESS_PROVIDERS`
  (commands.rs) — a missed site is a silent no-op in batch mode, and
  concurrent keychain hits can deadlock the macOS Security framework.
- Single-case Tauri commands are wrapped in `catch_command_panic`; any
  new run-style command must be too, or a panic freezes the UI.
- HTML5 drag/drop is broken in Tauri 2 WKWebView — the ClassifyDropDialog
  drag is Pointer-events based; don't "modernise" it back.
- UI icons are `@tabler/icons-react`, never emoji.

## Conventions

- Clippy `pedantic` + `nursery` at `-D warnings`; `thiserror` in lib
  crates, `anyhow` only in `cli`; `tracing`, no `println!`.
- Every user-facing string is bilingual: add keys to BOTH
  `apps/desktop/src/locales/es.json` and `en.json`.
- Conventional commits (`fix(scope): …`), matching `git log`.
- SQLite schema changes: idempotent `migrate_*` fns in
  `crates/verdict/src/persistence.rs` (PRAGMA table_info → ALTER TABLE),
  each with a legacy-schema test.

## Plans

`plans/` holds advisor-written implementation plans with a status index
(`plans/README.md`) — reconcile against it before re-auditing anything.
