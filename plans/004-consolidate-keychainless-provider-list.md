# Plan 004: Consolidate the keychain-less provider list into one helper so new providers can't silently no-op

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 4ecaaea..HEAD -- apps/desktop/src-tauri/src/commands.rs`
> If the file changed since this plan was written (Plans 001/002 also touch
> it), re-locate every site by the grep in "Current state" and compare shapes;
> on a structural mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW (pure consolidation; behavior must be byte-identical for existing ids)
- **Depends on**: 002 (same file; land after to avoid conflicts) — functionally independent
- **Category**: tech-debt
- **Planned at**: commit `4ecaaea`, 2026-06-10

## Why this matters

Six Tauri command paths each hardcode which providers authenticate outside
Conclave's keychain (Ollama, Apple Intelligence, the two OAuth providers, the
two CLI proxies). Adding a credential-less provider means updating every
site; missing one produces the worst kind of failure — in the batch path it
historically surfaced as a **silent no-op** (dialog closes, nothing runs, no
error), which cost a real debugging session (documented in this repo's
`.claude/napkin.md`: "There is no single helper — grep for `no API key for`
and confirm every site shares the same bypass arm"). This plan creates the
single helper so the next provider addition is a one-line change, and adds a
test that pins the list.

## Current state

All in `apps/desktop/src-tauri/src/commands.rs`. Find the sites with:

```
grep -n '"ollama" | "apple-intelligence"' apps/desktop/src-tauri/src/commands.rs
```

Five sites share this exact match (line numbers at commit `4ecaaea`):

| Line | Function | Shape |
|------|----------|-------|
| ~501 | `ask_documents` | `match request.provider_id.as_str() { LIST => String::new(), other => secrets::load(other)...ok_or_else(\|\| format!("no API key for {other}"))? }` |
| ~1022 | `test_provider` | same, but the error arm is `return err(format!("no API key for {id}"))` |
| ~2157 | `run_case_impl` | same as ask_documents, error text uses backticks: `` `{other}` `` |
| ~2521 | `run_draft_case` | same as run_case_impl |
| ~2698 | `run_case_deliberated_impl` | same as run_case_impl |

The list in all five: `"ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" | "claude-cli" | "codex-cli"`.

A sixth site differs (line ~1710, `preview_data_boundary`):

```rust
    let api_key = match request.provider_id.as_str() {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
        other => secrets::load(other)
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
    };
```

It omits the CLI ids but tolerates that via `unwrap_or_default()` (a missing
keychain entry yields `""` instead of an error — previews must never fail on
auth). Net effect today: previewing with `claude-cli`/`codex-cli` performs a
pointless keychain lookup but still works.

Context the comments at these sites preserve (keep this knowledge in the
helper's doc comment — it is load-bearing):

- comment at ~2150: calling `secrets::load` concurrently from batch workers
  triggered a macOS Security-framework deadlock — the bypass exists for
  correctness, not just convenience;
- comment at ~495: a missing bypass surfaces to the user as a silent no-op in
  batch mode.

`secrets` comes from `conclave_providers` (see the `use conclave_providers::{...}`
block at the top of commands.rs, line ~16).

Repo conventions: clippy `pedantic`+`nursery` `-D warnings`; comments are
full sentences explaining why.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Desktop tests | `cargo test -p conclave-desktop --locked --quiet` | exit 0 (substitute the package name from `apps/desktop/src-tauri/Cargo.toml` if different) |
| Format | `cargo fmt --all --check` | exit 0 |
| Whole gate | `./scripts/verify.sh` | "All local checks passed" |

## Scope

**In scope** (the only file you should modify):

- `apps/desktop/src-tauri/src/commands.rs`

**Out of scope** (do NOT touch):

- `crates/cli` — the CLI binary resolves keys its own way; not part of this
  finding.
- `build_provider` and provider construction — only the key *lookup* is being
  consolidated.
- `list_providers` (line ~738 uses `secrets::load(id).unwrap_or(None).is_some()`
  to display "configured" state) — different semantics (presence check, not
  retrieval); leave it.
- Frontend files.

## Git workflow

- Branch: `advisor/004-keychainless-helper`
- Commit style: `refactor(desktop): single source of truth for keychain-less providers`.
- Pre-commit hook runs `./scripts/verify.sh`; do not bypass.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the constant + two helpers

Near the top of commands.rs (after the `use` block, before the first
command), add:

```rust
/// Providers that authenticate outside Conclave's keychain: local
/// daemons (`ollama`), the on-device Apple bridge, OAuth providers
/// (tokens live in `<config>/oauth/*.json`), and the CLI proxies
/// (`claude-cli`, `codex-cli` — auth is the user's own CLI session).
///
/// Every run/test/Q&A path MUST consult this list instead of matching
/// inline. History: when `codex-cli` was added, one path missed the
/// bypass and "Ejecutar comité" became a silent no-op (the keychain
/// lookup failed before any event was emitted). Worse, `secrets::load`
/// fired concurrently from batch workers can deadlock in the macOS
/// Security framework — skipping it for these ids is a correctness
/// requirement, not an optimisation.
const KEYCHAIN_LESS_PROVIDERS: &[&str] = &[
    "ollama",
    "apple-intelligence",
    "anthropic-oauth",
    "openai-oauth",
    "claude-cli",
    "codex-cli",
];

fn provider_uses_keychain(id: &str) -> bool {
    !KEYCHAIN_LESS_PROVIDERS.contains(&id)
}

/// Resolve the API key for `provider_id`: empty string for
/// keychain-less providers, the stored key otherwise.
/// `Err` when a keychain-backed provider has no stored key.
fn resolve_provider_api_key(provider_id: &str) -> Result<String, String> {
    if !provider_uses_keychain(provider_id) {
        return Ok(String::new());
    }
    secrets::load(provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no API key for `{provider_id}`"))
}
```

Note the unified error text uses backticks (the majority form, 3 of 5 sites);
the two sites without backticks change their user-visible error trivially —
acceptable, the frontend treats these as opaque strings (verify with
`grep -rn "no API key" apps/desktop/src/` → expect no frontend matches; if
there ARE matches, STOP).

**Verify**: `cargo clippy --workspace --all-targets --locked -- -D warnings` → may flag dead code until Step 2; proceed then verify.

### Step 2: Replace the six sites

1. The five identical sites (`ask_documents`, `test_provider`,
   `run_case_impl`, `run_draft_case`, `run_case_deliberated_impl`):
   replace the whole `let api_key = match ... };` block with
   `let api_key = resolve_provider_api_key(&request.provider_id)?;`
   (in `test_provider` the id variable is `id` and errors return via
   `err(...)` — use `match resolve_provider_api_key(&id) { Ok(k) => k, Err(e) => return err(e) }`
   to preserve its return style).
   Keep each site's surrounding explanatory comment if it adds local context,
   but trim what the helper's doc now covers (the deadlock + silent-no-op
   story should live ONLY on the helper).
2. `preview_data_boundary` (~1710): replace its match with
   `let api_key = resolve_provider_api_key(&request.provider_id).unwrap_or_default();`
   This preserves "preview never fails on auth" AND fixes the needless
   keychain hit for CLI ids (they now short-circuit). Keep/adjust its local
   comment accordingly.

**Verify**:
`grep -c '"ollama" | "apple-intelligence"' apps/desktop/src-tauri/src/commands.rs` → exactly 0 outside the constant (the constant uses array syntax, so the grep should return 0).
`grep -c "no API key for" apps/desktop/src-tauri/src/commands.rs` → exactly 1 (inside the helper).
`cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0.

### Step 3: Pin the list with a test

In the commands.rs test module (or a new `#[cfg(test)] mod provider_key_tests`),
add:

```rust
#[test]
fn keychain_less_providers_resolve_to_empty_key() {
    for id in KEYCHAIN_LESS_PROVIDERS {
        assert_eq!(
            resolve_provider_api_key(id).as_deref(),
            Ok(""),
            "{id} must bypass the keychain"
        );
    }
}
```

This must not touch the real keychain — it can't, because the bypass returns
before `secrets::load`. Do NOT add a test for the keychain-backed arm (it
would hit the real macOS keychain in CI/pre-commit).

**Verify**: `cargo test -p conclave-desktop --locked --quiet` → exit 0, new test passes.

### Step 4: Full gate

**Verify**: `./scripts/verify.sh` → "✓ All local checks passed".

## Test plan

- `keychain_less_providers_resolve_to_empty_key` (Step 3).
- Everything else is covered by compilation + the grep done-criteria: the
  refactor's correctness property is "exactly one place encodes the list".

## Done criteria

ALL must hold:

- [ ] `./scripts/verify.sh` exits 0
- [ ] `grep -c "no API key for" apps/desktop/src-tauri/src/commands.rs` == 1
- [ ] `grep -n "KEYCHAIN_LESS_PROVIDERS" apps/desktop/src-tauri/src/commands.rs` shows: 1 definition, 1–2 helper uses, 1 test use
- [ ] No inline `"ollama" | "apple-intelligence" | ...` match arms remain (grep from Step 2)
- [ ] New test passes
- [ ] Only `apps/desktop/src-tauri/src/commands.rs` modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `grep -rn "no API key" apps/desktop/src/` returns frontend matches (the
  error string is load-bearing in UI logic — unifying the text would change
  behavior).
- You find a SEVENTH site matching on provider ids for key purposes that this
  plan doesn't list (e.g. added by Plans 001/002) — apply the helper there
  too ONLY if its semantics are identical; otherwise report.
- `set_provider_key` / `remove_provider_key` turn out to also need the list
  (napkin says they short-circuit for CLI ids) — check
  `grep -n "fn set_provider_key" -A 20 apps/desktop/src-tauri/src/commands.rs`:
  if they hardcode CLI ids for a *different* purpose (no-op on store/remove),
  leave them but NOTE it in your report as a candidate follow-up; do not
  force-fit the helper.

## Maintenance notes

- Adding a credential-less provider is now: append to
  `KEYCHAIN_LESS_PROVIDERS` (the pinned test self-documents). Reviewers of
  future provider PRs should check exactly that.
- If providers ever expose a capability flag like
  `requires_conclave_keychain()` on the `LlmProvider` trait, this constant
  can be retired in favor of asking the provider — deferred because key
  resolution currently happens *before* provider construction.
