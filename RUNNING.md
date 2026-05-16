# Running Conclave

This file walks through everything you need to launch the desktop app and
the CLI on macOS.

> Conclave is **not a medical device**. Anything that comes out of it is
> decision support, not a clinical decision. You accept the medical
> disclaimer the first time you launch the app; the CLI prints it on
> every invocation.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| Rust 1.82+ stable | builds every crate | `curl https://sh.rustup.rs -sSf \| sh` |
| Node 20+ + pnpm | the desktop frontend | `brew install node pnpm` |
| protoc | lance-encoding (a transitive of LanceDB) needs it | `brew install protobuf` |
| Xcode Command Line Tools | linker on macOS | `xcode-select --install` |

Optional:

- **Ollama** — for local LLM completions (`brew install ollama && ollama serve`).
- **Tesseract** — only if you want the `--features ocr` build (`brew install tesseract`).

## Quick start — desktop app

```bash
cd apps/desktop
pnpm install
pnpm tauri dev          # development, hot-reload UI
# or
pnpm tauri build        # production: produces .app + .dmg under
                        # apps/desktop/src-tauri/target/release/bundle/
```

On first launch the **onboarding modal** shows the medical disclaimer.
After accepting:

1. **Workspaces** — create one (`Cardiología`, etc.). It becomes active.
2. **Settings** — paste your Anthropic API key. It lands in the macOS
   keychain (service `Conclave`). Click *Test* to confirm.
   - If you prefer local-only inference, run `ollama serve` and pick
     the `ollama` provider instead — no key needed.
3. **Knowledge** — *Ingest file…* or *Ingest folder…* to load
   protocols / guidelines (PDF, DOCX, TXT, MD, HTML).
   First ingest downloads the embedding model (~470 MB) into
   `~/Library/Caches/fastembed`.
4. **Cases** — *New case* → paste a clinical note → *Preview de-id* to
   see what will leave your machine → *Run committee*. The verdict
   renders with cited evidence, certainty, red flags and follow-up
   triggers.
5. Optional: *Accept / Modify / Reject* feedback on the verdict for the
   learning loop.

## Quick start — CLI

```bash
cargo build --release            # ~5 min first time (LanceDB + ORT)
target/release/conclave-cli --help
```

Typical session:

```bash
conclave-cli workspace create "Cardiología"
conclave-cli providers set anthropic     # prompts for the key, stores in keychain
conclave-cli ingest path/to/your/protocols/
conclave-cli search "infarto agudo de miocardio" --k 5
conclave-cli case new --question "Manejo inicial de ICA" --file caso.txt
conclave-cli case list
conclave-cli feedback accept <case-id>
```

The CLI honours `CONCLAVE_WORKSPACE=<name>` and `--workspace <name>` to
target a non-default workspace.

## Privacy at a glance

- **De-identification is mandatory.** The verdict pipeline calls the
  Phase 3 PII pipeline (regex DNI/NIE/MRN/email/phone/date/age + name
  heuristics) before any LLM request. The CLI's `deident` subcommand
  and the desktop's *Preview de-id* button show exactly what will
  leave your device.
- **Secrets in keychain only.** API keys are never written to TOML
  files.
- **No telemetry.** The app never phones home.
- **Local file copies.** Every ingested document is copied into the
  workspace directory; we never read from the user's original location
  after that point.

## Data locations

- macOS: `~/Library/Application Support/me.badia.conclave/` (desktop)
  and `~/Library/Application Support/dev.Conclave.conclave/` (CLI).
- Linux: `~/.local/share/conclave/`.
- Windows: `%APPDATA%\Conclave\`.

Each workspace lives under `workspaces/<id>/` with `workspace.toml`,
`metadata.sqlite`, `cases.sqlite`, `vectors.lance/` and `documents/`.

## Troubleshooting

- **`protoc` not found.** `brew install protobuf` and re-run the build.
- **`fastembed` first ingest is slow.** It downloads ~470 MB of ONNX
  weights once into `~/Library/Caches/fastembed`. Subsequent runs are
  instant.
- **"401 / Auth" from the provider.** Run *Settings → Test* to confirm
  the stored key is correct.
- **Onboarding modal keeps appearing.** The acceptance marker lives at
  `~/Library/Application Support/me.badia.conclave/disclaimer-accepted-v1`;
  delete it to reset.

## What's deferred

These two phases land in follow-up PRs. The app is fully usable
without them:

- **Phase 5 — Learning loop (partial).** The feedback table is wired
  and the CLI persists `accept/modify/reject` rows, but the case-memory
  embedding + past-cases retrieval in the verdict prompt is not yet
  hooked up. To finish: embed `case_summary` into a `case_memory`
  LanceDB table on verdict completion, then surface the top-3 similar
  past cases in the prompt's `PAST_CASES` block (the placeholder is
  already in the template).
- **Phase 6 — Online evidence.** PubMed + Europe PMC adapters are not
  built. The verdict prompt already reserves the `[X*]` citation
  space; wiring is purely additive.
EOF
