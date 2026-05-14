# Conclave — Architecture

This document captures the **target shape** of Conclave and the road that gets
us there. It is the canonical reference for cross-crate boundaries and for
the data flow through the system.

## Target stack

```
┌──────────────────────────────────────────────────────────────────┐
│                       Desktop app (Tauri 2)                      │
│  ┌────────────────────────────┐    ┌──────────────────────────┐  │
│  │   React + TypeScript UI    │◀──▶│  Tauri command bridge    │  │
│  └────────────────────────────┘    └──────────────────────────┘  │
└─────────────────────────────────────────────┬────────────────────┘
                                              │
                                              ▼
┌──────────────────────────────────────────────────────────────────┐
│                        Rust core (this repo)                     │
└──────────────────────────────────────────────────────────────────┘
```

Phase 0 only ships the Rust core and a CLI; the Tauri shell lands once the
core is feature-complete and once a macOS dev box is available.

## Crate graph

```
                ┌─────────────┐
                │     cli     │  conclave-cli binary
                └──────┬──────┘
                       │ uses
        ┌──────────────┼──────────────────┐
        ▼              ▼                  ▼
  ┌──────────┐   ┌──────────┐       ┌──────────┐
  │   rag    │   │ providers│       │  deident │
  └────┬─────┘   └────┬─────┘       └────┬─────┘
       │              │                  │
       └──────────────┴────────┬─────────┘
                               ▼
                        ┌──────────────┐
                        │     core     │  shared types, errors,
                        └──────────────┘  config, paths, logging
```

- **`core`** is the only crate every other crate depends on. It owns the
  [`Error`](./crates/core/src/error.rs) type, the on-disk
  [`Config`](./crates/core/src/config.rs), the OS-aware
  [`Paths`](./crates/core/src/paths.rs), and the
  [`tracing`](./crates/core/src/logging.rs) initialiser.
- **`providers`** defines a single async [`Provider`](./crates/providers/src/lib.rs)
  trait plus its associated message/role/usage types. Concrete adapters
  (Anthropic, OpenAI, local llama.cpp, …) land in Phase 2.
- **`rag`** carves the Retrieval-Augmented Generation pipeline: ingestion,
  chunking, embedding, and hybrid search. Phase 0 only ships
  Unicode-safe chunking; the rest follows in Phase 1.
- **`deident`** carves the PII de-identification pipeline. Phase 0 ships
  the [`Deidentifier`](./crates/deident/src/lib.rs) trait and a
  conservative placeholder detector; real detectors arrive in Phase 3.
- **`cli`** wires everything together into `conclave-cli`, the testing
  entry point until the desktop UI exists.

## Data flow (target)

```
                ┌──────────────────────────────────────────────┐
                │              user / clinician                │
                └──────────────────────┬───────────────────────┘
                                       │  clinical question
                                       ▼
   ┌───────────┐   query    ┌──────────────────────────────┐
   │  deident  │◀───────────│         orchestrator         │
   └────┬──────┘            └────┬──────────────┬──────────┘
        │ redacted text          │ retrieved    │ provider
        ▼                        │ context      │ panel
   ┌──────────────────────────────┐              │
   │     rag (search & rerank)    │              │
   └────┬─────────────────────────┘              │
        │ top-k chunks                           │
        ▼                                        ▼
   ┌──────────────────────────────────────────────────────────┐
   │     committee deliberation (provider-by-provider)        │
   └────────────────────────────┬─────────────────────────────┘
                                │ structured verdict
                                ▼
                       ┌──────────────────┐
                       │   audit log      │
                       └──────────────────┘
```

Three rules govern this pipeline:

1. **Nothing reaches a provider before `deident` has run.** PII redaction is
   not optional.
2. **Every committee output is auditable.** Each verdict carries the
   prompts, retrieved chunks and per-provider opinions that produced it.
3. **No silent fallbacks on safety-critical paths.** Errors propagate
   through [`conclave_core::Error`](./crates/core/src/error.rs) and are
   surfaced to the user.

## On-disk layout

```
${CONFIG_DIR}/conclave.toml          # core::Config (TOML)
${DATA_DIR}/workspaces/<name>/       # per-workspace knowledge bases
${CACHE_DIR}/                        # regenerable indices, embeddings
```

`CONFIG_DIR`, `DATA_DIR` and `CACHE_DIR` are resolved by the
[`directories`](https://crates.io/crates/directories) crate via
[`Paths::resolve`](./crates/core/src/paths.rs).

## Phases

| Phase | Theme              | Status        | Deliverables                                            |
|-------|--------------------|---------------|---------------------------------------------------------|
| 0     | Foundations        | done          | Workspace, CI, config, logging, CLI skeleton            |
| 1     | Knowledge base     | **current**   | Ingestion (md/txt/pdf/html/docx), chunking, embeddings, hybrid BM25+dense search |
| 2     | Providers          | next          | Anthropic / OpenAI / local adapters, provider tests     |
| 3     | De-identification  | planned       | Real PII detectors, evaluation harness                  |
| 4     | Deliberation       | planned       | Multi-provider orchestrator, verdict format, audit log  |
| 5     | Desktop shell      | planned       | Tauri 2 app, React/TS UI                                |

## Phase 1 details (knowledge base)

The knowledge-base pipeline runs synchronously and is fully local:

1. **Loaders** (`crates/rag/src/loaders/`) turn each supported file into a
   normalised `Document` (path + format + optional title + plain text).
   Per-format details:
   - Markdown via `pulldown-cmark` (heading extracted as title).
   - Plain text verbatim.
   - PDF via `pdf-extract` (text-layer only — scanned PDFs need OCR, which
     is deferred to Phase 1.5).
   - HTML via `scraper` + `html2text` (scripts/styles dropped; `<title>`
     preserved).
   - DOCX via `docx-rs` (paragraph-level extraction).
2. **Chunking** (`crates/rag/src/chunking.rs`) splits text into overlapping
   Unicode-codepoint-aligned chunks, sized via `config.rag`.
3. **Embeddings** (`crates/rag/src/embeddings/`) implement the `Embedder`
   trait. Two backends:
   - `MockEmbedder` — deterministic, hash-seeded, used in tests and CI.
   - `FastEmbedEmbedder` (behind the `fastembed-backend` feature) —
     downloads ONNX models on first use into `${CACHE_DIR}/models/`.
4. **Store** (`crates/rag/src/store.rs`) — one SQLite file per workspace
   under `${DATA_DIR}/workspaces/<name>/knowledge.sqlite`. Schema:
   `documents` + `chunks` (with embedding BLOBs) + `chunks_fts`
   (FTS5 with `unicode61` tokenizer, diacritics folded). Dense similarity
   runs through a Rust-registered `cosine_sim(blob, ?)` SQL scalar; the
   blob layout is wire-compatible with a future `sqlite-vec` swap.
5. **Search** (`crates/rag/src/search.rs`) — three modes: `bm25`, `dense`,
   `hybrid`. Hybrid mode fetches `2 * top_k` candidates per leg and fuses
   them with Reciprocal Rank Fusion (`config.knowledge.{bm25_weight,
   dense_weight, rrf_k}`).
6. **Ingest** (`crates/rag/src/ingest.rs`) — walks a path, deduplicates by
   Blake3 of the normalised text, runs the full pipeline in a single SQLite
   transaction per document. Re-running ingestion on unchanged inputs is a
   no-op.
