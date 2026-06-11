# Plan 002: Catch panics in single-case Tauri commands so the UI shows an error instead of freezing

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 4ecaaea..HEAD -- apps/desktop/src-tauri/src/commands.rs`
> If the file changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW (additive wrapper; the pattern is already proven in the batch path)
- **Depends on**: none (if Plan 001 landed first, re-run the drift check — both touch commands.rs, but different functions)
- **Category**: bug
- **Planned at**: commit `4ecaaea`, 2026-06-10

## Why this matters

This repo has a documented production failure class: a panic inside the
case pipeline (most recently a UTF-8 char-boundary slice panic in
`crates/deident`, fixed in commit `4ecaaea`) kills the whole Tauri command
future. The batch runner was hardened — its per-case futures are wrapped in
`catch_unwind` so a panic degrades to a visible `CaseFailed` event
(commands.rs:3317). But the **single-case** commands (`run_case`,
`run_case_deliberated`, `run_draft_case`, `ask_documents`) have no such net:
a panic there means the `invoke()` promise on the frontend never settles, the
spinner spins forever, and the user force-quits the app with no error message
and no `Failed` status persisted. Any future panic bug — and the deident
class has already produced one — turns into a frozen UI instead of a readable
error. This plan applies the already-proven pattern to the four unprotected
commands.

## Current state

All in `apps/desktop/src-tauri/src/commands.rs` (~3,980 lines):

- `run_case` (line 1737) — thin wrapper:

  ```rust
  #[tauri::command]
  pub async fn run_case(
      app: tauri::AppHandle,
      state: State<'_, AppState>,
      request: CaseRunRequest,
  ) -> CommandResult<CaseRunResponse> {
      run_case_impl(&app, &state, request, None).await
  }
  ```

- `run_case_deliberated` (line 2671) — same shape, delegates to
  `run_case_deliberated_impl(&app, &state, request, None).await`.
- `run_draft_case` (line 2443) — `#[tauri::command]` whose body IS the
  implementation (no separate `_impl`); returns
  `CommandResult<CaseRunResponse>`; knows its case id up front
  (`request.case_id`).
- `ask_documents` (line 488) — `#[tauri::command]`, body is the
  implementation, returns `CommandResult<AskDocumentsResponse>`.
- `CommandResult<T>` is `Result<T, String>` (see its alias definition near
  the top of commands.rs).
- The proven pattern, in `run_batch_cases` (lines 3310–3343):

  ```rust
  match std::panic::AssertUnwindSafe(per_case).catch_unwind().await {
      Ok(outcome) => outcome,
      Err(panic) => {
          let msg = panic
              .downcast_ref::<&str>()
              .map(|s| (*s).to_owned())
              .or_else(|| panic.downcast_ref::<String>().cloned())
              .unwrap_or_else(|| "panicked".to_owned());
          tracing::error!(index = idx, error = %msg, "batch case panicked");
          ...
      }
  }
  ```

  `catch_unwind` on a future comes from `futures::FutureExt` (imported
  locally inside `run_batch_cases` as `use futures::FutureExt;`).
- `mark_case_failed_best_effort(store, case_id, err)` (line 2125) marks a
  case `Failed` + persists the error string; used by the existing error
  paths.

Repo conventions: clippy `pedantic` + `nursery` at `-D warnings`; tracing for
logs (no `println!`); comments explain *why*, in full sentences (see the
comment style around lines 495–500).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Typecheck/lint | `cargo clippy -p conclave-desktop --all-targets --locked -- -D warnings` | exit 0 |
| Tests | `cargo test -p conclave-desktop --locked --quiet` | exit 0 |
| Format | `cargo fmt --all --check` | exit 0 |
| Whole gate | `./scripts/verify.sh` | "All local checks passed" |

(If `-p conclave-desktop` is not the package name, find it with
`grep -m1 '^name' apps/desktop/src-tauri/Cargo.toml` and substitute.)

## Scope

**In scope** (the only file you should modify):

- `apps/desktop/src-tauri/src/commands.rs`

**Out of scope** (do NOT touch, even though they look related):

- `run_batch_cases` — already protected; do not refactor it to use the new
  helper in this plan (its panic arm emits batch events, a different shape).
- `ingest_path` / `ingest_paths` — extraction panics there are already
  contained by `extract_safely` in `crates/rag`; leave them.
- The frontend — no TS changes; the error string surfaces through the
  existing rejected-promise path.
- `crates/deident`, `crates/verdict` — fixing root-cause panics is not this
  plan's job; this plan is the safety net.

## Git workflow

- Branch: `advisor/002-panic-net-single-case`
- Commit style: conventional commits, e.g.
  `fix(desktop): catch panics in single-case commands so the UI gets an error`.
- Pre-commit hook runs `./scripts/verify.sh`; do not bypass it.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add a reusable panic-catching helper

In commands.rs, near `mark_case_failed_best_effort` (line ~2125), add:

```rust
/// Convert a panic anywhere inside a command future into a normal
/// `Err(String)` so the frontend's `invoke()` promise settles and the UI
/// can show an error instead of spinning forever. The batch runner has
/// carried the same net since the deident char-boundary panic froze a
/// batch (see `run_batch_cases`); this extends it to single-case runs.
async fn catch_command_panic<T>(
    label: &str,
    fut: impl std::future::Future<Output = CommandResult<T>>,
) -> CommandResult<T> {
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(result) => result,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panicked".to_owned());
            tracing::error!(command = label, error = %msg, "command panicked");
            Err(format!("internal error: {msg}"))
        }
    }
}
```

Match the exact downcast chain from `run_batch_cases` (excerpt above). Note
`AssertUnwindSafe` is justified for the same reason as in the batch path: the
future is consumed by value and never polled again after a panic.

**Verify**: `cargo clippy -p conclave-desktop --all-targets --locked -- -D warnings` → exit 0 (it will flag the helper as dead code until Step 2 — if so, proceed to Step 2 and verify after).

### Step 2: Wrap the four commands

1. `run_case` (line 1737): body becomes
   `catch_command_panic("run_case", run_case_impl(&app, &state, request, None)).await`.
2. `run_case_deliberated` (line 2671): same with
   `run_case_deliberated_impl(...)` and label `"run_case_deliberated"`.
3. `run_draft_case` (line 2443): its body is inline. Rename the existing
   function to `run_draft_case_impl` (drop the `#[tauri::command]` attribute
   from it, change visibility to `pub(crate) async fn` taking
   `state: &State<'_, AppState>` ONLY IF a mechanical borrow change is
   needed — prefer the minimal edit: keep its signature identical and add a
   new `#[tauri::command] pub async fn run_draft_case(...)` wrapper that
   forwards). Inside the wrapper, since the case id is known up front, on the
   panic arm also mark the case failed:

   ```rust
   let case_id = request.case_id.clone();
   let result = catch_command_panic("run_draft_case", run_draft_case_impl(state_ref, request)).await;
   if let Err(e) = &result {
       if e.starts_with("internal error:") {
           if let Ok(store) = case_store_arc(state_ref, &workspace_id) { /* mark failed */ }
       }
   }
   ```

   Simplification allowed: if threading `workspace_id` into the wrapper is
   awkward, capture it from `request.workspace_id` before the move. Use
   `mark_case_failed_best_effort(&store, &case_id, e)` exactly like the
   existing error paths do.
4. `ask_documents` (line 488): body is inline and has no case id. Rename to
   `ask_documents_impl` and add the thin wrapping command with label
   `"ask_documents"`. No failure-marking needed.

Keep each `_impl` function adjacent to its wrapper. Do not change any
function's externally visible Tauri name (the `#[tauri::command]` fn names
must stay `run_case`, `run_case_deliberated`, `run_draft_case`,
`ask_documents` — the frontend invokes them by string).

**Verify**: `cargo clippy -p conclave-desktop --all-targets --locked -- -D warnings` → exit 0.
`grep -c "catch_command_panic(" apps/desktop/src-tauri/src/commands.rs` → ≥ 5 (1 definition + 4 call sites).

### Step 3: Unit-test the helper

In the existing `#[cfg(test)]` module of commands.rs (find with
`grep -n "mod tests" apps/desktop/src-tauri/src/commands.rs`; if commands.rs
has none, add `#[cfg(test)] mod panic_net_tests { ... }` at the bottom),
add two `#[tokio::test]`s:

1. `panicking_future_becomes_err`: pass
   `async { panic!("boom {}", 42); #[allow(unreachable_code)] Ok::<(), String>(()) }`
   through `catch_command_panic("test", ...)`; assert the result is
   `Err(e)` with `e.contains("internal error")` and `e.contains("boom")`.
2. `ok_future_passes_through`: pass `async { Ok::<u8, String>(7) }`; assert
   `Ok(7)`.

**Verify**: `cargo test -p conclave-desktop --locked --quiet` → exit 0, both new tests pass.

### Step 4: Full gate

**Verify**: `./scripts/verify.sh` → "✓ All local checks passed".

## Test plan

- Two unit tests on `catch_command_panic` (Step 3) — the wrapper is the
  load-bearing logic; the four call sites are mechanical forwarding verified
  by compilation and the grep done-criterion.
- No frontend tests (the frontend already renders rejected invoke promises as
  error banners; nothing changed there).

## Done criteria

ALL must hold:

- [ ] `./scripts/verify.sh` exits 0
- [ ] `grep -c "catch_command_panic(" apps/desktop/src-tauri/src/commands.rs` ≥ 5
- [ ] `grep -n "#\[tauri::command\]" apps/desktop/src-tauri/src/commands.rs | wc -l` is unchanged from before your edit (no command added/removed — record the before count in your report)
- [ ] Both new unit tests exist and pass
- [ ] Only `apps/desktop/src-tauri/src/commands.rs` is modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The four functions don't match the line numbers/shapes in "Current state"
  (drift — likely Plan 001 landed; re-locate by name and re-confirm shape;
  if a function's body changed structurally, stop).
- `AssertUnwindSafe(...).catch_unwind()` fails to compile because a captured
  type is not `UnwindSafe` in a way the batch path doesn't hit — do not
  scatter `AssertUnwindSafe` deeper; report.
- Renaming `run_draft_case` → `run_draft_case_impl` breaks the
  `invoke_handler!` registration in `apps/desktop/src-tauri/src/lib.rs` or
  `main.rs` in a way that isn't fixed by registering the new wrapper name —
  the registered name must remain exactly `run_draft_case`.
- You find yourself wanting to modify frontend files — out of scope.

## Maintenance notes

- Any NEW `#[tauri::command]` that runs the pipeline (future Q&A modes,
  re-runs) must go through `catch_command_panic` — note this in review.
- A panic caught in `run_case`/`run_case_deliberated` leaves the staged draft
  row in its pre-run status and may leave a stale entry in
  `state.case_cancels` (benign: the entry is replaced on the next run of the
  same case id, and the map is small). If that map ever becomes
  user-visible, add cleanup in the panic arm.
- Root-cause hardening of the deident panic class is a separate concern
  (property tests over arbitrary UTF-8 in `crates/deident` — see
  plans/README.md "Deferred findings").
