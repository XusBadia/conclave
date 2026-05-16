#!/usr/bin/env bash
# Local verification — runs the same safety net that GitHub Actions used to
# give us, but on your own machine and for free.
#
# Invoked from `.githooks/pre-commit` on every `git commit`, and runnable
# directly any time:
#
#     ./scripts/verify.sh
#
# Steps:
#   1. cargo fmt --all --check
#   2. cargo clippy --workspace --all-targets --locked -- -D warnings
#   3. cargo test --workspace --locked --quiet
#   4. pnpm --dir apps/desktop build  (tsc -b && vite build)
#
# On a warm Cargo cache the whole thing takes ~3-5 minutes. First run after a
# `cargo clean` or pulling deep dep changes will take longer.

set -euo pipefail

# Resolve repo root so the script works from any cwd (including .git/hooks
# context, where cwd is the repo root anyway, but be safe).
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

step() {
  printf "\n\033[1;36m▶ %s\033[0m\n" "$*"
}

fail() {
  printf "\n\033[1;31m✗ %s\033[0m\n" "$*" >&2
  exit 1
}

# Bail early if the toolchains we need are missing — clearer than the bare
# "command not found" you'd get otherwise.
command -v cargo >/dev/null || fail "cargo not found in PATH"
command -v pnpm  >/dev/null || fail "pnpm not found in PATH"

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets --locked -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace --locked --quiet"
cargo test --workspace --locked --quiet

step "pnpm --dir apps/desktop build"
pnpm --dir apps/desktop build

printf "\n\033[1;32m✓ All local checks passed\033[0m\n"
