# Contributing to Conclave

Thanks for taking the time. Conclave is in **Phase 0** — the surface area is
intentionally small. This document captures the conventions that keep the
codebase consistent while it grows.

## Code of conduct

Be excellent to each other. Clinical software is sensitive — assume good
faith, default to kindness, escalate gently.

## Toolchain

- The toolchain is pinned in [`rust-toolchain.toml`](./rust-toolchain.toml)
  to the stable channel; `rustup` will install the right version on demand.
- The repository uses a Cargo workspace; run `cargo` commands from the
  repository root.

## Local checks

Before opening a PR, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

CI runs exactly these on Linux, macOS and Windows. Match local to CI.

## Coding conventions

- **Errors**: workspace crates surface failures through
  `conclave_core::Error`. Add new variants there when introducing a new
  fault domain.
- **No `unwrap` / `expect` in library code**, except in tests and in
  conditions documented as infallible at the call site.
- **No `unsafe`**. The workspace lint profile sets `unsafe_code = "forbid"`.
- **Logging**: use `tracing` macros. The CLI initialises the subscriber via
  `conclave_core::logging::init`. Don't call `tracing_subscriber` directly
  from library crates.
- **Config changes**: every field added to `conclave_core::Config` must
  ship a default, be covered by a round-trip test, and be documented in
  the README's quick-start section if user-visible.

## Commit style

Conventional-Commits-flavoured prefixes are encouraged:

```
feat(rag): hybrid BM25 + dense retriever
fix(deident): keep dosage numbers <4 digits
chore(ci): cache target/ across jobs
docs(readme): note Windows paths
```

Commits should be **atomic and self-contained**. A test added alongside a
fix is one commit; a refactor split into preparation + behaviour change is
two.

## Branches

Day-to-day work happens on feature branches off `main`. Phase work uses the
naming convention `claude/conclave-phase-<n>-<slug>` for branches authored
through the Claude Code workflow.

## Adding a new crate

1. Create `crates/<name>/Cargo.toml` mirroring the existing crates'
   `[package]` block (inherit `version`, `edition`, etc. from workspace).
2. Add the crate to `[workspace.members]` and (if it's a library) to
   `[workspace.dependencies]` so other crates can refer to it as
   `<name> = { workspace = true }`.
3. Wire `[lints] workspace = true` so the strict lint profile applies.
4. Run `cargo check --workspace` to confirm the graph still resolves.

## Releasing

Releases are deferred until Phase 4. The current contract is that `main`
must always be CI-green.
