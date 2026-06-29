# Conclave MD — Phased Plan

This is the canonical roadmap. Each phase has a clear deliverable and
acceptance criteria. Phases must be completed in order. No skipping ahead.

## Phase 0 — Foundations

**Goal:** clean repo, green CI on 3 OS, navigable empty skeleton.

- Cargo workspace with crates: `core`, `providers`, `rag`, `deident`, `cli`.
- `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `.editorconfig`.
- `tracing` + `tracing-subscriber` set up across crates.
- Config loading (TOML, OS-appropriate dirs via `directories`).
- `conclave-cli --help` shows placeholder subcommands.
- GitHub Actions CI: fmt + clippy (-D warnings) + test + build on
  ubuntu/macos/windows.
- Docs in `docs/`: README, ARCHITECTURE, PLAN, DISCLAIMER.

**Acceptance:**

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` passes.
- CI green on all 3 OS.
- `conclave-cli workspace --help` returns a useful message.

## Phase 1 — Knowledge Base

**Goal:** user can ingest, list, view and delete protocol documents.

- Document ingestion pipeline (PDF, DOCX, TXT, MD, HTML).
- OCR feature flag with `tesseract` for scanned PDFs.
- Semantic chunking with overlap (500-800 tokens, sentence-aware).
- Embeddings with `fastembed-rs` + `multilingual-e5-small`.
- LanceDB store + SQLite metadata in workspace directory.
- CLI subcommands:
  - `workspace create <name>`
  - `ingest <path>` (single file or directory)
  - `documents list`, `documents show <id>`, `documents remove <id>`
  - `search "query"` — returns top-K chunks with snippets.

**Acceptance:**

- Ingest 50 mixed documents, total < 5 min on a normal machine.
- Search returns relevant chunks with stable IDs.
- Removing a document drops its chunks from the vector store.
- Integration test: ingest fixture → search → expected chunk in top 3.

## Phase 2 — Provider Layer

**Goal:** unified inference layer with all three modes.

- `LlmProvider` trait + first implementations:
  - `anthropic_api`
  - `openai_api`
  - `openrouter_api`
  - `ollama_local`
- Secret storage via `keyring` crate.
- CLI subcommands:
  - `providers list`
  - `providers set <id>` (interactive: asks for key, stores in keychain)
  - `providers test <id>` (sends a hello, prints latency + model)
- Routing config: separate model for light tasks vs reasoning.

**Acceptance:**

- All four providers can complete a hello-world call.
- Keys are not visible in any config file on disk.
- Switching providers does not require rebuilding.
- Mock provider available for tests (no network).

### Phase 2.5 — OAuth Providers (optional, can skip if risky)

- `anthropic_oauth` (Claude Max only, with extra credits).
- `openai_codex_oauth` (ChatGPT Plus/Pro/Business).
- Clear “experimental” labelling in UI.
- ToS disclaimer shown on activation.

## Phase 3 — De-identification

**Goal:** zero PII leaves the device without explicit user consent.

- NER pipeline: rule-based + small NER model (multilingual).
- Detection categories listed in `ARCHITECTURE.md`.
- Masking with stable tokens (`<PATIENT_NAME_1>`, etc.).
- CLI: `deident "text"` returns `{original, spans, masked}` as JSON.
- Strict mode: errors if obvious PII patterns remain in masked output.

**Acceptance:**

- 95%+ recall on a small Spanish + English clinical text test set.
- Zero false negatives on document IDs (NHC, MRN, DNI, NIE).
- Output is reproducible (same input → same tokens).

## Phase 4 — Verdict Engine

**Goal:** the heart of the product. Case text → structured recommendation.

- Retrieval: top-K chunks from workspace + optional re-rank.
- Case memory: similar previous cases retrieved as few-shot examples.
- Rules injection: workspace rules always in system prompt.
- Prompt assembly module with templates in `docs/PROMPTING.md`.
- Generation with strict JSON schema output:
  - case_summary
  - key_clinical_data
  - applied_evidence (with citations to documents)
  - primary_recommendation
  - alternatives
  - certainty_level (high/medium/low) + justification
  - red_flags
  - disclaimer
- Citation resolution: each evidence reference links back to document + page.
- CLI: `case new --file caso.txt` runs full pipeline, prints verdict.

**Acceptance:**

- 3 golden cases produce verdicts that pass schema validation.
- Citations resolve to actual document chunks.
- De-identification step cannot be bypassed.
- Verdict JSON is stable enough that small input changes produce small
  output changes (sanity, not exact reproducibility).

## Phase 5 — Learning Loop

**Goal:** the app improves with use.

- Persist for each case: input (masked), retrieval, prompt, output,
  feedback (accept/modify/reject + reason).
- Similarity search for cases when a new case comes in.
- Inject top-2 similar past cases (with their feedback) as examples.
- CLI: `feedback accept|modify|reject <case-id>`.
- Metrics: acceptance rate, most-cited documents, most-modified sections.
- Export workspace dataset as JSON for offline analysis.

**Acceptance:**

- After 10 cases with feedback, similar new case retrieves them as
  examples and verdict reflects user preferences encoded in feedback.

## Phase 6 — Online Evidence

**Goal:** complement local KB with live literature.

- PubMed E-utilities adapter.
- Europe PMC fallback.
- Local cache (SQLite) of queries and abstracts.
- Per-case toggle: “include online evidence search”.
- LLM-generated MeSH query from case text.
- Returned evidence labelled clearly as “external, not validated”.
- Requires network + a network-capable provider; blocked otherwise.

**Acceptance:**

- For a sample case, returns relevant recent papers.
- Verdict cites external evidence distinctly from internal protocols.
- Works offline gracefully (toggle disabled, clear message).

## Phase 7 — Desktop UI

**Goal:** macOS-grade Tauri app on top of the now-stable core.

- Tauri 2 scaffold.
- React + TS + Tailwind + shadcn/ui.
- Sections: Workspaces, Knowledge, Cases, Settings.
- macOS-first: vibrancy, integrated title bar, native sidebar feel.
- Document preview (pdf.js).
- Verdict view with collapsible sections and clickable citations.
- Onboarding flow with disclaimer modal.
- Command palette (⌘K).

**Acceptance:**

- Every CLI capability is also reachable from the UI.
- App passes basic accessibility checks (keyboard nav, focus order).
- Looks native on macOS to a non-expert user.

## Phase 8 — Distribution

- macOS: notarised universal DMG.
- Windows: signed MSI.
- Linux: AppImage + .deb.
- Auto-update via `tauri-plugin-updater`.
- Landing page.
- Public docs site.

## Out of scope for v1

- Device sync.
- Multi-user shared workspaces.
- HL7/FHIR.
- CE-MDR certification.
- Voice input.
- Image (radiology) analysis.
