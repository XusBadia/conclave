# Plan 001: Make PHI attachment retention honest — truthful audit records, UI disclosure, and an opt-in auto-purge

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat 4ecaaea..HEAD -- crates/verdict/src/persistence.rs crates/verdict/src/pipeline.rs crates/core/src/config.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src/lib/ipc.ts apps/desktop/src/routes/Settings.tsx apps/desktop/src/routes/Cases.tsx`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED (touches PHI lifecycle; a bug here either deletes files users wanted or keeps files users asked to discard)
- **Depends on**: none
- **Category**: security (privacy invariant)
- **Planned at**: commit `4ecaaea`, 2026-06-10

## Why this matters

Conclave is a clinical decision-support app whose core promise (ARCHITECTURE.md, "Privacy invariants") is that raw patient data is handled conservatively and that the audit trail tells the truth. Today, when a case runs with `retain_raw_text = false`, the app purges the raw clinical **text** from SQLite and writes `raw_text_retention = "discarded"` into the `audit_runs` table — but the **original attachment files** (PDFs, DOCX, photos: the rawest PHI there is) silently remain on disk under the case's `attachments/` directory. Batch runs hardcode `retain_raw_text: false`, so *every* batch case writes an audit row implying full discard while its source documents persist indefinitely. For a clinician relying on the audit log to answer "what patient data does this machine still hold?", the record is currently misleading.

The intended behavior is partly deliberate — attachments stay viewable in the case detail UI, and there are manual purge buttons — so the fix is **honesty plus opt-in automation**, not silent deletion:

1. Audit rows record whether attachment files were retained.
2. The pre-run data-boundary disclosure tells the user attachments are kept on disk.
3. A new privacy setting (default **off**, preserving current behavior) auto-purges attachment files whenever raw text is not retained.

## Current state

Files and roles:

- `crates/verdict/src/persistence.rs` — SQLite `CaseStore`: schema, migrations, `AuditRunRecord`, `purge_case_phi` (line 694), `purge_case_attachment_files` (line 858).
- `crates/verdict/src/pipeline.rs` — quick-mode verdict pipeline; `VerdictOptions` (line ~70), post-run purge block (lines 641–643).
- `crates/core/src/config.rs` — `PrivacyConfig` (line 87) with `default_data_boundary`; validation at line ~177.
- `apps/desktop/src-tauri/src/commands.rs` — Tauri commands: deliberated-run purge block (lines 3043–3045), batch request construction hardcoding `retain_raw_text: false` (line ~3261), `DataBoundaryPreview` struct (line 1441) and `boundary_preview_for_request` (line 1473), `PrivacySettingsDto` + `privacy_settings` + `set_privacy_settings` (lines 1628–1652), manual purge commands `purge_case_phi` / `purge_case_attachments` (lines 3576–3603).
- `apps/desktop/src/lib/ipc.ts` — frontend types: `DataBoundaryPreview` (`sends_images` line 308, `stores_raw_text` line 309), `PrivacySettings` (line 209).
- `apps/desktop/src/routes/Settings.tsx` — loads privacy settings (line 97), saves them (line 307).
- `apps/desktop/src/routes/Cases.tsx` — renders the boundary disclosure before a run (lines 2374–2381).

Key excerpts as of commit `4ecaaea`:

`crates/verdict/src/pipeline.rs:640-643` (quick-mode purge — text only):

```rust
            store.mark_case_status(&case.id, CaseStatus::ReviewReady)?;
            if !options.retain_raw_text {
                store.purge_case_phi(&case.id)?;
            }
```

`apps/desktop/src-tauri/src/commands.rs:3043-3045` (deliberated-mode purge — text only):

```rust
        if !request.retain_raw_text {
            g.purge_case_phi(&case_id).map_err(|e| e.to_string())?;
        }
```

`crates/verdict/src/persistence.rs:694` — `purge_case_phi` only does
`UPDATE cases SET original_text = '', raw_text_sha256 = ?, raw_text_retention = 'discarded'`.
It never touches files.

`crates/verdict/src/persistence.rs:858-889` — `purge_case_attachment_files`
already exists and does exactly what we need: iterates
`list_attachments_for_case`, `std::fs::remove_file(att.stored_path)`
(tolerating NotFound), then zeroes `stored_path`/`byte_size`/`needs_ocr` on
each `case_attachments` row, returning the purged count. It is currently
called **only** from the manual Tauri command `purge_case_attachments`
(commands.rs:3591) and the CLI (`crates/cli/src/commands/case.rs:334`).

`apps/desktop/src-tauri/src/commands.rs:3025-3029` (deliberated-mode audit row):

```rust
            raw_text_retention: if request.retain_raw_text {
                case.raw_text_retention
            } else {
                RawTextRetention::Discarded
            },
```

`crates/verdict/src/persistence.rs:371-394` — `AuditRunRecord` fields end with:

```rust
    pub attachment_refs: Vec<String>,
    pub raw_text_retention: RawTextRetention,
    pub status: String,
    pub error: Option<String>,
}
```

`apps/desktop/src-tauri/src/commands.rs:1497-1507` — `boundary_preview_for_request` returns:

```rust
    DataBoundaryPreview {
        mode,
        provider_id: provider.id().to_owned(),
        provider_requires_network,
        sends_masked_text: true,
        sends_raw_text: false,
        sends_images,
        stores_raw_text: request.retain_raw_text,
        uses_online_evidence: request.use_online_evidence,
        blocked_reason,
    }
```

`apps/desktop/src-tauri/src/commands.rs:1628-1631`:

```rust
pub struct PrivacySettingsDto {
    pub default_data_boundary: DataBoundaryMode,
}
```

`apps/desktop/src/routes/Cases.tsx:2374-2381` (the pre-run disclosure that
must learn about attachment retention):

```tsx
                  images: boundaryPreview.sends_images
                  ...
                    `cases.raw_retention.${boundaryPreview.stores_raw_text ? "explicit_retained" : "discarded"}`,
```

Repo conventions that apply:

- Migrations: follow the existing idempotent pattern — query
  `PRAGMA table_info(<table>)`, check for the column, `ALTER TABLE ... ADD
  COLUMN ... DEFAULT ...`. Exemplars: `migrate_retrieval_traces_attachments`
  (persistence.rs:447), `migrate_cases_privacy` (persistence.rs:529). Each has
  a matching `#[test]` (e.g. `migration_marks_legacy_raw_text_retained`)
  that creates a legacy schema and asserts the migration fills defaults — copy
  that structure.
- Clippy runs with `pedantic` + `nursery` at `-D warnings` (workspace lints in
  root `Cargo.toml`). Write lint-clean code; `missing_docs` is allowed but
  public items in these crates generally carry `///` docs — match that.
- Error handling: library crates use `thiserror` `Result`; commands.rs maps to
  `Result<T, String>` via `.map_err(|e| e.to_string())`.
- Frontend strings are bilingual via i18next. Find the locale files with
  `grep -rn "raw_retention" apps/desktop/src` and add both `es` and `en`
  entries for every new key. UI affordances use `@tabler/icons-react`, never
  emojis.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Rust tests (verdict crate) | `cargo test -p conclave-verdict --locked --quiet` | exit 0 |
| Full Rust gate | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all --check` | exit 0 |
| Frontend build + typecheck | `pnpm --dir apps/desktop build` | exit 0 (`tsc -b && vite build`) |
| Whole gate (pre-commit equivalent) | `./scripts/verify.sh` | "All local checks passed" |

## Scope

**In scope** (the only files you should modify):

- `crates/verdict/src/persistence.rs` (schema, migration, `AuditRunRecord`, insert/read of audit rows, tests)
- `crates/verdict/src/pipeline.rs` (`VerdictOptions` + purge block + inline tests)
- `crates/core/src/config.rs` (`PrivacyConfig` field + default + tests if present for config round-trip)
- `apps/desktop/src-tauri/src/commands.rs` (audit-row construction, purge blocks, `DataBoundaryPreview`, `PrivacySettingsDto`, privacy commands)
- `apps/desktop/src/lib/ipc.ts` (type updates only)
- `apps/desktop/src/routes/Settings.tsx` (one toggle in the privacy section)
- `apps/desktop/src/routes/Cases.tsx` (one disclosure line near line 2381)
- The i18next locale files (es + en) you locate via grep

**Out of scope** (do NOT touch, even though they look related):

- `crates/cli/src/commands/case.rs` — the CLI's manual `purge` subcommands already behave correctly; leave them.
- The manual Tauri commands `purge_case_phi` / `purge_case_attachments` (commands.rs:3576–3603) — they stay as-is.
- Batch UI flow in Cases.tsx beyond the single disclosure line — do not add per-batch retention pickers in this plan.
- `run_batch_cases`'s hardcoded `retain_raw_text: false` (commands.rs:~3261) — leave the hardcode; the new setting governs what `false` *means*.

## Git workflow

- Branch: `advisor/001-phi-attachment-retention`
- Commit style: conventional commits matching `git log` (e.g. `fix(verdict): record attachment retention in audit runs`). One commit per step is fine.
- The repo's pre-commit hook runs `./scripts/verify.sh` (~3–5 min warm). Do NOT bypass with `--no-verify`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Record attachment retention truthfully in `audit_runs`

In `crates/verdict/src/persistence.rs`:

1. Add `pub attachments_retained: bool,` to `AuditRunRecord` (after
   `raw_text_retention`, line ~391).
2. Add column `attachments_retained INTEGER NOT NULL DEFAULT 1` to the
   `audit_runs` CREATE TABLE statement (the block containing
   `raw_text_retention TEXT NOT NULL DEFAULT 'discarded'`, ~line 139).
   Default `1` (= retained) is the honest default for legacy rows: files were
   kept.
3. Add `fn migrate_audit_runs_attachments_retained(conn: &Connection)`
   following the `migrate_cases_privacy` pattern exactly (PRAGMA
   table_info → ALTER TABLE ADD COLUMN), and call it where the other
   `migrate_*` functions are invoked in `CaseStore::open`.
4. Thread the field through `insert_audit_run` and every place audit rows are
   read back (search `raw_text_retention` within persistence.rs to find the
   SELECT/row-mapping sites; mirror its handling).
5. Update the two `AuditRunRecord` construction sites:
   - `crates/verdict/src/pipeline.rs:610-633` — set
     `attachments_retained: true` for now (step 3 revisits it).
   - `apps/desktop/src-tauri/src/commands.rs:~2990-3032` — same.
6. Add a migration test modeled on the existing migration tests in
   persistence.rs (create a legacy `audit_runs` table without the column,
   open the store, assert the column exists and legacy rows read back as
   `attachments_retained == true`).

**Verify**: `cargo test -p conclave-verdict --locked --quiet` → exit 0, new
migration test passes. `cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0.

### Step 2: Add the privacy setting `purge_attachments_with_raw_text`

1. `crates/core/src/config.rs` — add to `PrivacyConfig` (line 87):
   `pub purge_attachments_with_raw_text: bool,` with `#[serde(default)]` so
   existing `config.toml` files load unchanged, and `false` in the
   `Default` impl (line 92). No validation needed (it's a bool).
2. `apps/desktop/src-tauri/src/commands.rs` — extend `PrivacySettingsDto`
   (line 1628) with `pub purge_attachments_with_raw_text: bool,`; read it in
   `privacy_settings` from `cfg.privacy.purge_attachments_with_raw_text`;
   write it in `set_privacy_settings` alongside `default_data_boundary`.
3. `apps/desktop/src/lib/ipc.ts` — add the field to `PrivacySettings`
   (line 209).
4. `apps/desktop/src/routes/Settings.tsx` — in the privacy block that loads
   (line ~97) and saves (line ~307) the settings, add a labeled toggle for the
   new field, following the visual pattern of the existing privacy control
   next to it. Add i18n keys (es + en) — e.g.
   `settings.privacy.purge_attachments.label` and `.help`, with help text
   stating plainly: "When raw text is discarded after a run, also delete the
   original attachment files from disk." (Spanish equivalent for `es`.)

**Verify**: `pnpm --dir apps/desktop build` → exit 0.
`cargo test -p conclave-core --locked --quiet` → exit 0 (config default/round-trip tests still pass).

### Step 3: Wire the auto-purge into both run paths

1. `crates/verdict/src/pipeline.rs` — add
   `pub purge_attachment_files: bool,` to `VerdictOptions` (struct at ~line
   70, `false` in its `Default`). In the purge block (lines 641–643), extend:

   ```rust
   if !options.retain_raw_text {
       store.purge_case_phi(&case.id)?;
       if options.purge_attachment_files {
           store.purge_case_attachment_files(&case.id)?;
       }
   }
   ```

   And set `attachments_retained` in the `AuditRunRecord` built ~30 lines
   above to the truthful value:
   `!(!options.retain_raw_text && options.purge_attachment_files)` — i.e.
   `attachments_retained` is `false` only when the purge will actually run.
   Compute it once into a local `let attachments_retained = ...;` above the
   record construction so the record and the purge can't disagree.
2. `apps/desktop/src-tauri/src/commands.rs` — in every place a
   `VerdictOptions` is built for a run (search `options.retain_raw_text =`;
   there are three: `run_case_impl` ~line 2228, `run_draft_case` ~line 2566,
   and the deliberated path), set
   `options.purge_attachment_files = cfg.privacy.purge_attachments_with_raw_text;`
   (each site already clones `cfg` from `state.config`).
3. The deliberated path persists its own audit row and purge block directly in
   commands.rs (not through `VerdictPipeline::run_for_case`). Mirror the same
   logic there: in the block at lines 3043–3045 add the conditional
   `purge_case_attachment_files` call, and set the audit row's
   `attachments_retained` (step 1's site at ~3025) from the same local.
4. Add a pipeline test in `crates/verdict/src/pipeline.rs`'s inline test
   module (model on the existing `happy_path_end_to_end_with_mocks`): run a
   case with a fake attachment file on disk, `retain_raw_text: false`,
   `purge_attachment_files: true`, then assert (a) the file is gone, (b) the
   attachment row's `stored_path` is empty, (c) the audit row has
   `attachments_retained == false`. Add a second test with
   `purge_attachment_files: false` asserting the file survives and the audit
   row says `true`.

**Verify**: `cargo test -p conclave-verdict --locked --quiet` → exit 0 including the two new tests.

### Step 4: Disclose attachment retention in the pre-run boundary preview

1. `apps/desktop/src-tauri/src/commands.rs` — add
   `pub retains_attachment_files: bool,` to `DataBoundaryPreview` (line 1441)
   and set it in `boundary_preview_for_request` (line ~1497):
   `!request.attached_file_paths.is_empty() && (request.retain_raw_text || !purge_setting)`.
   `boundary_preview_for_request` doesn't currently see the config — give it
   the bool as a new parameter (`purge_attachments_with_raw_text: bool`) and
   update its callers (`preview_data_boundary`, `run_case_impl`,
   `run_draft_case`, `run_case_deliberated_impl` — find them with
   `grep -n "boundary_preview_for_request" apps/desktop/src-tauri/src/commands.rs`),
   each reading the flag from the same `cfg` they already load (for
   `preview_data_boundary`, lock `state.config` the same way
   `privacy_settings` does).
2. `apps/desktop/src/lib/ipc.ts` — add `retains_attachment_files: boolean;`
   to `DataBoundaryPreview` (line ~308).
3. `apps/desktop/src/routes/Cases.tsx` — next to the existing
   `cases.raw_retention.*` line (~2381), add one disclosure line shown when
   `boundaryPreview.retains_attachment_files` is true, e.g. i18n key
   `cases.boundary.attachments_retained`: EN "Original attachment files stay
   on this Mac until you purge them." / ES equivalent. Match the surrounding
   markup style exactly; no new components.

**Verify**: `pnpm --dir apps/desktop build` → exit 0.

### Step 5: Full gate

**Verify**: `./scripts/verify.sh` → "✓ All local checks passed".

## Test plan

- `crates/verdict/src/persistence.rs`: migration test for
  `attachments_retained` column (legacy table → migrated, default true).
  Model: the existing `migration_*` tests in the same file.
- `crates/verdict/src/pipeline.rs`: two end-to-end mock-provider tests (purge
  on / purge off) as described in Step 3.4. Model:
  `happy_path_end_to_end_with_mocks` in the same file.
- Manual (report, don't automate): none required; the desktop UI change is a
  toggle + a text line, covered by `tsc`/build.

## Done criteria

ALL must hold:

- [ ] `./scripts/verify.sh` exits 0
- [ ] `cargo test -p conclave-verdict --locked --quiet` exits 0 with ≥3 new tests (1 migration + 2 pipeline)
- [ ] `grep -n "attachments_retained" crates/verdict/src/persistence.rs apps/desktop/src-tauri/src/commands.rs` shows the field in schema, struct, both insert sites
- [ ] `grep -n "purge_attachments_with_raw_text" crates/core/src/config.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src/lib/ipc.ts apps/desktop/src/routes/Settings.tsx` hits all four layers
- [ ] `grep -n "purge_case_attachment_files" crates/verdict/src/pipeline.rs apps/desktop/src-tauri/src/commands.rs` shows the call inside both `!retain_raw_text` blocks
- [ ] New i18n keys exist in BOTH the `es` and `en` locale files
- [ ] No files outside the in-scope list are modified (`git status`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The purge blocks at `pipeline.rs:641` or `commands.rs:3043` don't match the
  excerpts above (drift).
- You cannot find where `cases.raw_retention.*` i18n keys live after
  `grep -rn "raw_retention" apps/desktop/src` — the disclosure step depends on it.
- The deliberated-run path turns out to ALSO call
  `VerdictPipeline::run_for_case` (it should not — it has its own persistence
  block); if it does, the double-purge logic needs rethinking.
- Adding the `boundary_preview_for_request` parameter forces signature changes
  in more than the four named callers.
- Any test reveals that `purge_case_attachment_files` deletes files that are
  still referenced by another case (shared attachment rows) — that would be a
  data-loss bug; report instead of shipping.

## Maintenance notes

- Future work that adds per-batch retention choices (the batch path hardcodes
  `retain_raw_text: false` at commands.rs:~3261) should reuse
  `purge_attachments_with_raw_text` rather than adding a second flag.
- Reviewer should scrutinize: the truthfulness coupling in Step 3 (one local
  feeding both the audit row and the purge decision), and that legacy audit
  rows read back as `attachments_retained = true` (not false).
- Deferred deliberately: encrypting or shredding (vs unlinking) purged files;
  purging the deident span map; per-case retention overrides in batch mode.
