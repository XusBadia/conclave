# Phase 0 — Foundations

Paste this to Claude Code when starting Phase 0. Reference docs live at
the repo root: `README.md`, `ARCHITECTURE.md`, `PLAN.md`, `DISCLAIMER.md`,
`PROMPTING.md`, `CONTRIBUTING.md`.

---

I'm Xus. We're building Conclave, a desktop clinical decision support app
that acts as a virtual multidisciplinary board. Read `README.md`,
`ARCHITECTURE.md`, `PLAN.md` and `CONTRIBUTING.md` before starting. They
are the authoritative spec.

I'm working from iPhone via Claude Code right now, no Mac available until
this weekend. We start with backend Rust + CLI, no Tauri yet. UI comes
later in Phase 7.

## What to build in Phase 0

Set up the foundations exactly as `ARCHITECTURE.md` describes.

1. **Cargo workspace** at the repo root with these crates:
   - `crates/core`
   - `crates/providers`
   - `crates/rag`
   - `crates/deident`
   - `crates/cli` (binary, `conclave-cli`)

   Each crate has a minimal `lib.rs` (or `main.rs` for cli) with a doc
   comment describing its responsibility per the architecture doc.

2. **Toolchain and lints**
   - `rust-toolchain.toml` pinning stable.
   - `rustfmt.toml` with sensible defaults (max_width 100, edition 2024,
     reorder_imports, group_imports = "StdExternalCrate").
   - `clippy.toml` enabling pedantic-but-not-painful settings.
   - `.editorconfig` with UTF-8, LF, 4-space Rust, 2-space TS/JSON.

3. **Workspace Cargo.toml**
   - `resolver = "2"`.
   - Shared `[workspace.lints.rust]` and `[workspace.lints.clippy]`
     applied to all crates.
   - Shared dependency versions in `[workspace.dependencies]`.
   - Initial shared deps: `tracing`, `tracing-subscriber`, `thiserror`,
     `anyhow`, `serde`, `serde_json`, `tokio` (with `rt-multi-thread`,
     `macros`), `directories`, `toml`, `clap` (with `derive`),
     `async-trait`. Pick recent stable versions.

4. **`core` crate**
   - `error.rs`: `ConclaveError` enum with `thiserror`, variants for
     `ConfigError`, `IoError`, `SerdeError`. Result type alias.
   - `config.rs`: `Config` struct (serde), default + load + save.
     Uses `directories` crate to find the right dir per OS.
     Path: `<data_dir>/Conclave/config.toml`.
   - `tracing.rs`: `init_tracing()` that sets up `tracing-subscriber`
     with `EnvFilter::from_default_env().or("info")`, pretty formatter,
     no JSON for now.
   - Domain type stubs (just the structs with `Serialize/Deserialize`):
     `Workspace`, `Document`, `Chunk`, `Case`, `Verdict`, `Feedback`,
     `Rule`. Fields per architecture doc; don't overdesign, we'll
     iterate.
   - Tests: config load/save round-trip; default config has expected
     paths.

5. **Other crates stubs**
   - `providers/lib.rs`: empty `LlmProvider` trait skeleton with the
     signatures from the architecture doc.
   - `rag/lib.rs`: empty `Ingester` placeholder.
   - `deident/lib.rs`: empty `Deidentifier` placeholder.
   - Each with one trivial test so they compile and CI exercises them.

6. **`cli` crate**
   - `clap` with subcommands as named in the architecture doc, all
     returning `unimplemented!()` for now with a clear message.
   - Top-level command initialises tracing and prints the legal
     disclaimer once per process on first invocation (track via a
     marker file in config dir; if already shown, skip).
   - `conclave-cli --version` works.
   - `conclave-cli workspace --help`, `ingest --help`, etc. all
     respond with a useful help text.

7. **GitHub Actions CI** at `.github/workflows/ci.yml`
   - Trigger on push to `main` and on PRs.
   - Matrix: `ubuntu-latest`, `macos-latest`, `windows-latest`.
   - Steps: checkout, install stable toolchain with rustfmt + clippy,
     restore cargo cache (`Swatinem/rust-cache@v2`), `cargo fmt
     --check`, `cargo clippy --all-targets --all-features -- -D
     warnings`, `cargo test --all-features`, `cargo build --release`.
   - Job name should make the OS obvious.

8. **License**: MIT. Add `LICENSE` file at repo root with my name "Xus"
   and current year.

## Quality bar

- `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test` must pass locally and in CI on all three OS.
- No `unwrap()` outside tests. No `expect()` without an invariant
  comment.
- Every public item has at least a one-line rustdoc.
- Commits atomic, conventional commits style (`feat(core): add config
  loader`, etc.).
- Push directly to `main`.

## How to work

- Plan the structure first, write it out as a checklist in the first
  message of this session.
- Implement in this order: workspace + toolchain → core → providers/rag/
  deident stubs → cli → CI → docs check.
- Commit after each substep with a clear message.
- When done, run the full check locally and report what passed.
- Open a tracking issue in GitHub named "Phase 0 — Foundations" with a
  checklist mirroring the items above; tick each as you go.

When everything is green on CI in all three OS, ping me. Then we move to
Phase 1.
