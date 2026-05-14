# Conclave

> Virtual multidisciplinary clinical committee, on your desk.

Conclave is a desktop application — eventually built on **Tauri 2** with a
**Rust** core and a **React + TypeScript** UI — that orchestrates a panel of
large language models acting as a virtual multidisciplinary committee
(*comité multidisciplinar*) to help clinicians stress-test a question, plan
or differential against their own knowledge base.

## Status

**Phase 0 — Foundations.** This repository currently contains:

- A Rust [Cargo workspace](./Cargo.toml) with five crates: `core`,
  `providers`, `rag`, `deident` and `cli`.
- A `conclave-cli` binary used as the testing entry point until the
  desktop UI is built (no Mac on hand yet).
- Strict workspace-wide lints (`clippy::pedantic` + `nursery` + `cargo`),
  formatting via `rustfmt`, unit tests, and a 3-OS CI matrix.

Phase 1 (Knowledge Base) is next — see `ARCHITECTURE.md` for the roadmap.

## Medical disclaimer

**Conclave is an experimental clinical decision-support assistant. It is NOT
a medical device and does NOT replace the judgement of a qualified
clinician.** Outputs may be incomplete, biased, or wrong. Always validate
any suggestion against primary sources and institutional protocols before
acting on it.

The same disclaimer is printed by `conclave-cli` on every invocation; pass
`--no-disclaimer` to suppress it (e.g. in scripted contexts).

## Quick start

```bash
# Build everything
cargo build --workspace

# See the CLI surface
cargo run -p conclave-cli -- --help

# Initialise a workspace under a custom root (for testing)
cargo run -p conclave-cli -- \
    --workspace-root ./.conclave-dev \
    workspace init

# Inspect resolved paths and effective config
cargo run -p conclave-cli -- \
    --workspace-root ./.conclave-dev \
    workspace info
```

The default workspace root follows your operating system's conventions:

| Platform | Config dir                                              |
|----------|---------------------------------------------------------|
| Linux    | `$XDG_CONFIG_HOME/conclave/` (typically `~/.config/conclave/`) |
| macOS    | `~/Library/Application Support/dev.Conclave.conclave/`  |
| Windows  | `%APPDATA%\Conclave\conclave\config\`                   |

## Project layout

```
crates/
  core/         shared types, error, config, paths, logging
  providers/    LLM provider trait + (later) concrete implementations
  rag/          ingestion, chunking, embeddings, search
  deident/      PII de-identification for clinical text
  cli/          conclave-cli binary (testing entry point)
```

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the data flow and the planned
phases.

## Development

Required toolchain is pinned in [`rust-toolchain.toml`](./rust-toolchain.toml)
to the stable channel. Common commands:

```bash
cargo fmt --all                     # format
cargo clippy --all-targets -- -D warnings   # lint
cargo test --workspace              # tests
cargo run -p conclave-cli -- --help # CLI
```

CI runs `fmt`, `clippy`, `test` and `build` on Ubuntu, macOS and Windows.

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for contributor guidelines.

## License

Dual-licensed under either of:

- Apache License, Version 2.0, ([LICENSE-APACHE](./LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
