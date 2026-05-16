# Phase 1 — Knowledge Base

Pre-requisite: Phase 0 complete, CI green on all 3 OS.

---

Phase 0 is done. Now Phase 1: the Knowledge Base. Re-read
`ARCHITECTURE.md` (the `rag` crate section) and `PLAN.md` (Phase 1)
before starting. Implement strictly to spec.

## What to build

Implement document ingestion, embedding, storage and retrieval. CLI-only,
no UI.

### 1. Document extraction

In `crates/rag`, build text extractors for these formats:

- **PDF**: try `pdf-extract` first, fallback to `lopdf` if extraction
  returns empty. If both produce empty text, mark the document as
  "needs OCR" and skip embedding (do not crash). OCR comes next.
- **DOCX**: use `docx-rs` or `dotext`. Whatever works reliably.
- **TXT / MD**: read as UTF-8, normalise line endings.
- **HTML**: strip tags with `scraper` or `html2text`. Keep the rendered
  text only, no markup.

Return a normalised `ExtractedText { content: String, page_breaks:
Vec<usize>, source_path: PathBuf, doc_type: DocType }`.

### 2. OCR (feature-gated)

Add an optional `ocr` feature. When enabled and a PDF needs OCR:
- Rasterise each page to a 300 DPI image with `pdfium-render` or
  similar.
- Run `tesseract-rs` with `spa+eng` languages.
- Stitch results back into `ExtractedText`.

If the feature is off, OCR is skipped and the document is flagged for
the user to convert manually.

### 3. Chunking

Implement sentence-aware semantic chunking:

- Split text into sentences (use a regex-based splitter that handles
  Spanish punctuation; `unicode-segmentation` for word boundaries).
- Greedily pack sentences into chunks targeting 500-800 tokens.
- Token count via `tiktoken-rs` (cl100k_base is fine as approximation,
  it's only used for sizing).
- Overlap: ~100 tokens between adjacent chunks. Don't break sentences
  to fit overlap.
- Output: `Vec<Chunk { id, text, document_id, page_start, page_end,
  position }>`.

### 4. Embeddings

- Use `fastembed-rs` with model `multilingual-e5-small`.
- Initialise the model lazily on first use and cache it for the
  process lifetime.
- Batch embeddings (32 at a time is a good default).
- Embedding dimension stored in metadata so future model changes are
  detectable.

### 5. Storage

Per workspace, under `<data_dir>/Conclave/workspaces/<workspace-id>/`:

- `metadata.sqlite` — created with `rusqlite` or `sqlx` (pick one and
  stick to it; `rusqlite` is simpler for embedded).
  Tables:
  - `documents` (id, source_path, copied_path, title, doc_type, sha256,
    ingested_at, page_count, status)
  - `chunks` (id, document_id, position, page_start, page_end, text)
  - `tags` (document_id, tag) — many-to-many helper
- `vectors.lance/` — LanceDB store with one table named `chunks`,
  columns: `id` (str), `embedding` (vector<f32>), `document_id` (str),
  `text` (str). Indexed for ANN search.
- `documents/` — copy of every ingested file (original bytes), filename
  `<sha256-prefix>-<original-name>`. We work on the copy, never on the
  user's original location.

### 6. CLI subcommands (wire them up)

- `conclave-cli workspace create <name> [--specialty <s>] [--language
  <iso>]`
- `conclave-cli workspace list`
- `conclave-cli workspace switch <name>`  (stores "active workspace" in
  config)
- `conclave-cli workspace delete <name> --confirm`
- `conclave-cli ingest <path>...` — accepts files or directories; if
  directory, recurse. Print a progress line per document with status.
- `conclave-cli documents list [--tag <t>] [--type <t>]`
- `conclave-cli documents show <id>` — prints metadata + first 500
  chars + chunk count.
- `conclave-cli documents remove <id>` — removes copy, metadata rows
  and vectors atomically.
- `conclave-cli search "query" [--k 10]` — embeds query, runs ANN
  search, prints top-K with: document title, page range, similarity
  score, snippet (first 200 chars of chunk).

All commands respect `--workspace <name>` to override the active one
for a single invocation.

### 7. Tests

- Unit tests for each extractor with fixture files in
  `crates/rag/tests/fixtures/` (small public-domain samples; no
  patient data).
- Integration test: ingest a fixture PDF + DOCX + TXT into a temp
  workspace, search for known content, expect specific chunk in top-3.
- Round-trip test: ingest → remove → list returns empty.

### 8. Performance smoke

Add a `cargo bench` or a `--time` flag on `ingest` that prints elapsed
time. We want < 5 min for 50 documents on a normal machine. Don't
over-optimise; just don't be obviously slow.

## Quality bar

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test` green on all 3 OS.
- CI matrix already includes the `ocr` feature OFF; add a separate job
  that runs Linux-only with `--features ocr` to ensure it at least
  compiles.
- All error paths return typed `ConclaveError` variants; bubble up
  cleanly to CLI.

## How to work

- Plan first, post the checklist.
- Implement extraction → chunking → embeddings → storage → CLI →
  tests, in that order.
- Commit per subsystem with conventional commit messages.
- Tracking issue: "Phase 1 — Knowledge Base".

When `conclave-cli search` reliably returns sensible results from a
small test corpus, we move to Phase 2.
