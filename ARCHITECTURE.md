# Conclave — Architecture

This document describes the structural decisions behind Conclave. It is the
authoritative reference for how the codebase should be organised. When in
doubt, this file wins over ad-hoc choices.

## Goals

- **Local-first.** Nothing leaves the user’s machine without explicit consent.
- **Provider-agnostic.** Swap LLM backends without touching domain logic.
- **Testable without a Mac.** The core, CLI and most logic must be runnable
  in a headless Linux sandbox.
- **UI as a thin layer.** UI is added later on top of a complete core; it
  must never own business logic.

## Workspace layout

The project is a Cargo workspace. Each crate has a single responsibility and
a narrow public surface.

```
conclave/
├── Cargo.toml              # workspace manifest
├── rust-toolchain.toml     # pinned stable
├── rustfmt.toml
├── clippy.toml
├── .editorconfig
├── .github/
│   └── workflows/
│       └── release.yml     # release bundles on v* tags (no push CI — see §CI)
├── .githooks/
│   └── pre-commit          # runs scripts/verify.sh before every commit
├── scripts/
│   └── verify.sh           # fmt + clippy + tests + frontend build + vitest
├── crates/
│   ├── core/               # shared types, errors, config, logging
│   ├── providers/          # LlmProvider trait + implementations
│   ├── rag/                # ingestion, chunking, embeddings, retrieval
│   ├── deident/            # PII detection and masking
│   ├── verdict/            # verdict engine: pipeline, deliberation, persistence, skills
│   ├── evidence/           # PubMed / Europe PMC adapters + SQLite cache
│   └── cli/                # conclave-cli binary
├── apps/
│   ├── desktop/            # Tauri 2 app (React frontend + src-tauri commands)
│   └── web/                # Next.js marketing site
├── plans/                  # advisor-written implementation plans + status index
├── ARCHITECTURE.md         # this file (docs live at the repo root)
├── PLAN.md                 # phased roadmap
├── PROMPTING.md            # prompt templates and rationale
└── DISCLAIMER.md           # full legal disclaimer text
```

## Crate responsibilities

### `core`

- Domain types: `Workspace`, `Document`, `Chunk`, `Case`, `Verdict`,
  `Feedback`, `Rule`, `Provider`, `ModelChoice`.
- Error types using `thiserror`. One `ConclaveError` enum at the crate root.
- Config loading/saving (`directories` crate to find OS-appropriate paths).
- Tracing setup (`tracing` + `tracing-subscriber`).
- No I/O of its own beyond config. No HTTP. No LLM logic.

### `providers`

- Public trait:
  
  ```rust
  #[async_trait]
  pub trait LlmProvider: Send + Sync {
      fn id(&self) -> &str;
      fn capabilities(&self) -> ProviderCapabilities;
      fn requires_network(&self) -> bool;
      async fn complete(&self, req: CompletionRequest)
          -> Result<CompletionResponse, ProviderError>;
  }
  ```
- Implementations (separate modules):
  - `anthropic_api` — API key, calls `api.anthropic.com`.
  - `openai_api` — API key, calls `api.openai.com`.
  - `openrouter_api` — API key, routes to many models.
  - `anthropic_oauth` — Claude Max OAuth (phase 2.5, optional).
  - `openai_codex_oauth` — ChatGPT Plus/Pro OAuth (phase 2.5, optional).
  - `ollama_local` — `localhost:11434`, no auth.
  - `apple_intelligence` — sidecar Swift binary (added with macOS UI work).
- Secret storage via `keyring` crate. Never write secrets to disk in plain.
- Each impl is feature-gated so embedded builds can drop unused providers.

### `rag`

- Document ingestion pipeline:
1. Type detection.
1. Text extraction (PDF, DOCX, TXT, MD, HTML).
1. OCR fallback for scanned PDFs (`tesseract`, optional feature).
1. Semantic chunking with overlap.
1. Embedding (default `multilingual-e5-small` via `fastembed-rs`).
1. Persistence to LanceDB + SQLite metadata.
- Retrieval:
  - Vector search top-K.
  - Optional re-ranking (LLM filter or cross-encoder later).
  - Returns chunks with source document, page, and snippet.
- Reindex command: re-embed everything in a workspace.

### `deident`

- NER over text using a lightweight on-device model (start with
  rule-based + small NER model, e.g. `gliner` quantised or similar
  multilingual option; final choice in Phase 3).
- Detects: names, surnames, dates, exact ages, document IDs
  (DNI/NIE/NHC/MRN), addresses, phone numbers, emails, centre identifiers.
- Output: original text + list of spans + masked text with stable tokens
  (`<PATIENT_NAME_1>`, `<DATE_1>`, etc.).
- Pure function. Never makes network calls. Never persists anything.

### `evidence` (Phase 6)

- PubMed E-utilities adapter.
- Europe PMC fallback.
- Local SQLite cache of queries and abstracts.
- Returns a uniform `EvidenceSnippet` struct.

### `cli`

- `conclave-cli` binary built with `clap`.
- Subcommands:
  - `workspace create|list|switch|delete`
  - `ingest <path>` — add document(s) to current workspace
  - `documents list|show|remove`
  - `rules add|list|remove` *(not yet implemented — workspace rules have
    no management surface today; `VerdictOptions.rules_block` exists but
    nothing populates it. See plans/README.md § Direction findings.)*
  - `case new` — accepts text via stdin or `--file`, runs full pipeline
  - `case list|show <id>`
  - `feedback accept|modify|reject <case-id>`
  - `providers list|set|test`
  - `config show|set <key> <value>`
- Output: human-readable by default, `--json` for machine output.

### `desktop` (Phase 7+)

- Tauri 2 app. Wraps the core via Tauri commands.
- Owns nothing the CLI cannot do. UI is a presentation layer.

## Data storage

All data is stored under the OS-appropriate user data directory:

- macOS: `~/Library/Application Support/Conclave/`
- Linux: `~/.local/share/conclave/`
- Windows: `%APPDATA%\Conclave\`

Structure inside that directory:

```
Conclave/
├── config.toml             # global config
├── workspaces/
│   └── <workspace-id>/
│       ├── workspace.toml  # workspace config + rules
│       ├── documents/      # copies of ingested files
│       ├── metadata.sqlite # documents, cases, feedback
│       └── vectors.lance/  # LanceDB store
└── cache/
    └── evidence/           # PubMed cache
```

## Logging

- `tracing` everywhere. No `println!` outside `main.rs`.
- Default level: `info`. `RUST_LOG=conclave=debug` for verbose.
- CLI uses pretty formatting; CI/structured logs use JSON.

## Error handling

- Library crates: `thiserror` for typed errors.
- Binary crates (`cli`): `anyhow` at the top level only.
- Never `unwrap()` outside tests. `expect()` only with a clear invariant
  message.

## Testing

- Unit tests inline in each crate (`#[cfg(test)]` modules).
- Integration tests in each crate’s `tests/` directory.
- E2E tests in `crates/cli/tests/` that exercise the full ingest → verdict
  pipeline with a mock provider.
- Golden cases (Phase 4+): canonical input → expected verdict structure.
  Stored as JSON fixtures.

## CI / verification

**No CI runs on push or PR.** Day-to-day verification is local-first,
enforced by the versioned pre-commit hook. Activate it once per clone:

```sh
git config core.hooksPath .githooks
```

Every `git commit` then runs `./scripts/verify.sh` (also runnable on
demand):

1. `cargo fmt --all --check`
1. `cargo clippy --workspace --all-targets --locked -- -D warnings`
1. `cargo test --workspace --locked --quiet`
1. `pnpm --dir apps/desktop build` (`tsc -b && vite build`)
1. `pnpm --dir apps/desktop test` (vitest)

GitHub Actions fires only on `v*` release tags (`release.yml`), building
the macOS-arm64 / Linux / Windows bundles as a draft Release.

## Privacy invariants

These are non-negotiable and must be enforced by code:

1. **No network calls** are made by `core`, `rag`, `deident`. Only by
   `providers` and `evidence`.
1. **De-identification is mandatory** before any prompt is built that
   contains patient text. The verdict pipeline must call `deident` first
   and must persist the masked version, not the raw.
1. **Secrets in keychain only.** No tokens or API keys in config files.
1. **No telemetry.** Conclave does not phone home. Period.

## Future considerations (not now)

- Sync between devices (iCloud Drive folder or git-style).
- Multi-user workspaces with a self-hosted server.
- HL7/FHIR connectors.
- CE-MDR certification path (separate regulatory project).
