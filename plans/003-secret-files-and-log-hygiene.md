# Plan 003: Write OAuth token files with owner-only permissions and stop logging evidence queries

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 4ecaaea..HEAD -- crates/providers/src/oauth_flow.rs crates/providers/src/anthropic_oauth.rs crates/evidence/src/pubmed.rs crates/evidence/src/europe_pmc.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW (tightens file modes and removes a log field; no behavior change for the app itself)
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `4ecaaea`, 2026-06-10

## Why this matters

Conclave's architecture doc says "Secrets in keychain only" — but OAuth
access/refresh tokens (Anthropic and OpenAI subscription auth) are, by
design, persisted to JSON files under the config dir. Those files are
created with `std::fs::write`, which inherits the process umask: on a default
macOS/Linux setup that's mode `0644` — world-readable. A refresh token is a
long-lived credential for the user's paid LLM subscription; any other local
user or process that can read the file can hijack it. The fix is to create
these files owner-read/write only (`0600`), the same convention the official
CLIs use for their credential files.

Separately, the PubMed evidence adapter logs the search query at `debug`
level. Queries are built from de-identified text, so this is hygiene rather
than an active leak — but for a clinical app, clinical query content does not
belong in log files that outlive the session. Drop the query field from the
log line.

## Current state

Files and roles:

- `crates/providers/src/oauth_flow.rs` — shared OAuth plumbing.
  `persist_tokens` (lines 521–578) writes:
  - line 533: `std::fs::write(&path, body)` → `<config_dir>/oauth/<provider_id>.json`
    (Conclave's own token store; **created by Conclave**, this is the
    primary fix target);
  - lines 552 and 570: best-effort mirrors into `~/.claude/.credentials.json`
    and `~/.codex/auth.json` — these only write **if the file already
    exists** (`cli_path.exists()` guard), and `fs::write` truncates in place,
    preserving whatever permissions the official CLI set. Leave the
    mirror writes' logic alone, but harden them too if trivial (see Step 1).
- `crates/providers/src/anthropic_oauth.rs` — line 515:
  `std::fs::write(path, body)` inside a function that serializes
  `claude_ai_oauth` credentials (access + refresh token). Find the enclosing
  `fn` with `grep -n "fn " crates/providers/src/anthropic_oauth.rs | awk -F: '$2 < 515' | tail -3`
  and read it; it is a credentials write and gets the same treatment.
  (Line 615 in the same file is inside `#[cfg(test)]` — leave tests alone.)
- `crates/evidence/src/pubmed.rs` — line ~213:
  `tracing::debug!(query = trimmed, "pubmed cache hit")` (exact shape may
  vary slightly; find with `grep -n "query = " crates/evidence/src/*.rs`).
- `crates/evidence/src/europe_pmc.rs` — check for the same pattern with the
  same grep; treat any hit identically.

Repo conventions:

- Workspace lints: clippy `pedantic` + `nursery`, `-D warnings`. Unix-only
  code must be `#[cfg(unix)]`-gated so the Windows release build (CI builds
  Windows on tag push) still compiles.
- Errors in `crates/providers` use `ProviderError::Other(String)` — see the
  surrounding `map_err` calls in `persist_tokens`.
- `tempfile` is already a workspace dev-dependency (used in providers tests).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Providers tests | `cargo test -p conclave-providers --locked --quiet` | exit 0 |
| Evidence tests | `cargo test -p conclave-evidence --locked --quiet` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all --check` | exit 0 |
| Whole gate | `./scripts/verify.sh` | "All local checks passed" |

## Scope

**In scope** (the only files you should modify):

- `crates/providers/src/oauth_flow.rs`
- `crates/providers/src/anthropic_oauth.rs`
- `crates/evidence/src/pubmed.rs`
- `crates/evidence/src/europe_pmc.rs` (only if the grep finds a query-logging line)

**Out of scope** (do NOT touch):

- `keyring` usage and API-key storage — already keychain-backed; not part of
  this finding.
- `crates/providers/src/openai_oauth.rs` token logic beyond what
  `persist_tokens` already covers (if you find an independent `fs::write` of
  token material there outside tests, report it in your summary instead of
  expanding scope).
- The OAuth flows themselves (PKCE, ports, state) — no protocol changes.
- Log lines that don't contain query/credential content.

## Git workflow

- Branch: `advisor/003-secret-file-perms`
- Commit style: conventional, e.g.
  `fix(providers): create OAuth token files with 0600 permissions`.
- Pre-commit hook runs `./scripts/verify.sh`; do not bypass.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add a `write_secret_file` helper and use it for all token writes

In `crates/providers/src/oauth_flow.rs`, add a private helper near
`persist_tokens`:

```rust
/// Write credential material to `path` with owner-only permissions.
///
/// `std::fs::write` would inherit the process umask (typically 0644 —
/// world-readable), which is the wrong default for refresh tokens. On
/// Unix the file is created 0600 *before* any bytes land in it; if the
/// file already exists its mode is tightened first so a pre-existing
/// world-readable file doesn't stay that way. On Windows, default ACLs
/// already scope the user profile directory; plain write is acceptable.
fn write_secret_file(path: &std::path::Path, body: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if path.exists() {
            let mut perms = std::fs::metadata(path)?.permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o600);
            std::fs::set_permissions(path, perms)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(body.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body)
    }
}
```

Make it `pub(crate)` so `anthropic_oauth.rs` can use it too. Then replace:

1. `oauth_flow.rs:533` — `std::fs::write(&path, body)` →
   `write_secret_file(&path, &body)` (keep the existing
   `.map_err(... ProviderError::Other ...)` wrapper).
2. `oauth_flow.rs:552` and `:570` — the two mirror writes; keep their
   `let _ = ...;` best-effort semantics:
   `let _ = write_secret_file(&cli_path, &serde_json::to_string_pretty(&payload).unwrap_or_default());`
3. `anthropic_oauth.rs:515` — same replacement (import the helper:
   `use crate::oauth_flow::write_secret_file;` — adjust the module path to
   match how the two modules already reference each other; check with
   `grep -n "use crate::" crates/providers/src/anthropic_oauth.rs`).

**Verify**: `cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0.
`grep -n "std::fs::write" crates/providers/src/oauth_flow.rs crates/providers/src/anthropic_oauth.rs` → only hits inside `#[cfg(test)]` modules (or none).

### Step 2: Unit-test the file mode

In `oauth_flow.rs`'s existing `#[cfg(test)]` module (find with
`grep -n "mod tests" crates/providers/src/oauth_flow.rs`; create one if
absent), add a Unix-gated test:

```rust
#[test]
#[cfg(unix)]
fn secret_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    // Fresh file → 0600.
    write_secret_file(&path, "{\"t\":1}").unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    // Pre-existing loose file → tightened on rewrite.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_secret_file(&path, "{\"t\":2}").unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}
```

**Verify**: `cargo test -p conclave-providers --locked --quiet` → exit 0, new test passes.

### Step 3: Drop query content from evidence logs

Run `grep -rn "query = " crates/evidence/src/`. For every `tracing::...`
hit that logs the query string (pubmed.rs:~213 confirmed; europe_pmc.rs to
be checked), remove the `query = ...` field but keep the event, e.g.
`tracing::debug!("pubmed cache hit")`. Do NOT remove non-logging uses of
`query` (function params, struct fields, URL building).

**Verify**: `grep -rn "query = trimmed\|query = %\|query = query" crates/evidence/src/` → no `tracing` hits.
`cargo test -p conclave-evidence --locked --quiet` → exit 0.

### Step 4: Full gate

**Verify**: `./scripts/verify.sh` → "✓ All local checks passed".

## Test plan

- `secret_files_are_owner_only` (Step 2): fresh-file mode + tighten-on-rewrite,
  Unix-only. Model the test structure on the existing tests at the bottom of
  `oauth_flow.rs` / `anthropic_oauth.rs` (they already use `tempfile`).
- Evidence crates: existing tests must stay green (they cover cache behavior;
  the log line carries no assertions).

## Done criteria

ALL must hold:

- [ ] `./scripts/verify.sh` exits 0
- [ ] `grep -rn "std::fs::write" crates/providers/src/ | grep -v "cfg(test)" | grep -v "mod tests" -A0` shows no token-material writes outside tests (manual check: each remaining hit is in a test module)
- [ ] New `secret_files_are_owner_only` test exists and passes
- [ ] `grep -rn "query = " crates/evidence/src/ | grep tracing` → empty
- [ ] No files outside the in-scope list are modified (`git status`)
- [ ] `plans/README.md` status row updated
- [ ] Your final report notes that ALREADY-WRITTEN token files on user
      machines keep their old mode until next login — recommend the operator
      mention `chmod 600 ~/Library/Application\ Support/*onclave*/oauth/*.json`
      in release notes, and treat any token that sat world-readable as worth
      re-issuing (log out / log in)

## STOP conditions

Stop and report back (do not improvise) if:

- `persist_tokens` no longer matches the excerpt (drift).
- The enclosing function at `anthropic_oauth.rs:515` turns out NOT to write
  credential material (read it first — if it writes something else, report
  instead of changing it).
- `OpenOptionsExt`/`PermissionsExt` imports fail on the pinned toolchain
  (rust 1.82) — they shouldn't; if they do, report rather than switching to
  a post-write `set_permissions`-only approach (that leaves a window where
  the file is world-readable with content).
- You find token material being **logged** anywhere while working — report
  it; do not expand scope silently.

## Maintenance notes

- Any future provider that persists tokens to disk must use
  `write_secret_file` — call this out in review so it becomes the convention.
- The mirror-writes into `~/.claude/.credentials.json` / `~/.codex/auth.json`
  intentionally remain best-effort and existence-gated; if the official CLIs
  ever change schema, that code breaks silently by design (`let _ =`).
- Deferred: migrating OAuth tokens into the OS keychain entirely (would
  align with the ARCHITECTURE.md invariant but changes the CLI-mirroring
  feature; product decision needed).
