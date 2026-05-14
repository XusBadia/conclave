# Contributing

This is a solo-dev project for now. These guidelines are for future me and
for AI coding agents working in the repo.

## Workflow rules

- Commit early, commit often. Atomic commits, conventional messages.
- Format: `<type>(<scope>): <subject>` — e.g., `feat(rag): add semantic chunker`.
- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`.
- Push directly to `main` while pre-alpha; switch to PR flow before v0.1.
- Each phase from `docs/PLAN.md` is one logical unit. Open a tracking issue
  with the phase’s acceptance criteria as the checklist.

## Code standards

- `cargo fmt` before commit. `rustfmt.toml` is authoritative.
- `cargo clippy -- -D warnings` must pass. If a lint is wrong, justify
  the `allow` in code with a comment.
- No `unwrap()` outside tests. No `expect()` without an invariant comment.
- New public items must have rustdoc with at least one usage example.
- Tests for new logic. Aim for behaviour tests over implementation tests.

## Privacy invariants (do not break)

These are enforced by CI grep and by careful review:

1. `core`, `rag`, `deident` crates must not depend on `reqwest`, `hyper`,
   or any HTTP client. Network access is centralised in `providers` and
   `evidence`.
1. No `println!` of patient-derived text in any binary. Use `tracing` at
   `trace` or `debug` level, which is gated by `RUST_LOG`.
1. No telemetry. No analytics SDKs. No phone-home.
1. Secrets must go through `keyring`. Searching the repo for plausible
   API key patterns must return zero matches.

## Adding a new LLM provider

1. Create `crates/providers/src/<name>.rs`.
1. Implement `LlmProvider` trait.
1. Add feature flag in `crates/providers/Cargo.toml`.
1. Add to provider registry in `crates/providers/src/lib.rs`.
1. Add a mock-network test.
1. Document in `docs/ARCHITECTURE.md` under provider list.

## Working with Claude Code

When delegating work, always include:

- The relevant phase doc section (`docs/PLAN.md`).
- The relevant architecture section (`docs/ARCHITECTURE.md`).
- Concrete acceptance criteria.
- Instruction to commit + push when done.

Prefer small chunks over big ones. Claude Code is much better at coherent
500-line tasks than 2000-line tasks.
