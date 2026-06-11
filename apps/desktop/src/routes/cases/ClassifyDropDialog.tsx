// ClassifyDropDialog and its satellites, moved verbatim from Cases.tsx.
// HARD CONSTRAINT (learned the hard way): the card-to-card drag uses
// Pointer events, NOT the HTML5 drag/drop API — WKWebView under Tauri 2
// strips custom MIME types and drops dragover/drop events. Do not
// "modernise" this back to `draggable`. No setPointerCapture either:
// it would pin elementFromPoint to the source chip.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { IconAlertTriangle, IconGripVertical, IconX } from "@tabler/icons-react";

import { Button } from "../../components/Button";
import { Textarea } from "../../components/Field";
import { cn } from "../../lib/cn";
import {
  ipc,
  usableProviders,
  type BatchCaseInput,
  type ProviderInfo,
  type Workspace,
} from "../../lib/ipc";
import { isClinicalEligible, preferredProvider } from "../../lib/providers";
import { ModeSelector, ProviderOfflineBanner } from "./banners";
import { attachmentBadgeColor } from "./helpers";
import { ProviderField } from "./ProviderField";

// ---------------------------------------------------------------------------
// ClassifyDropDialog — modal opened when the clinician drops ≥ 2 files on
// the Cases window. Renders the heuristic grouping (from
// `propose_case_grouping`) as editable patient cards with native HTML5
// drag/drop between cards, and offers two terminal actions:
//   • "Guardar como borradores" → `create_draft_cases` (no run)
//   • "Ejecutar comité (N)"     → `run_batch_cases` (creates + runs)
// ---------------------------------------------------------------------------

// Drag/drop between cards uses Pointer events instead of the HTML5 drag/drop
// API. WKWebView's protected-mode handling of `dataTransfer` is unreliable in
// Tauri 2 (custom MIME types are stripped from `dataTransfer.types` during
// dragover, and `dragover`/`drop` events frequently never reach the target
// element underneath the drag image). Pointer events bypass that entire
// pipeline and work consistently inside the WebView. The page-level Tauri
// `onDragDropEvent` listener for OS-file drops is unaffected because it
// operates at the NSView level, not the HTML5 event level.

type ClassifyDragState = {
  /** Index of the source row the chip came from. */
  fromRow: number;
  /** Index of the chip inside the source row. */
  fileIdx: number;
  /** File path — captured at pointerdown for the floating ghost label. */
  filePath: string;
  /** Pointer coordinates at pointerdown (for the 5 px activation threshold). */
  startX: number;
  startY: number;
  /** Live pointer position (drives the ghost's transform). */
  cursorX: number;
  cursorY: number;
  /** True once the cursor has moved > 5 px from (startX, startY). Below this
   *  threshold we treat the gesture as a click and never commit a move, so
   *  the chip's "Quitar" button keeps working. */
  activated: boolean;
  /** Which drop target the cursor is currently over, or `null` if none. */
  hoveringTarget: number | "new" | null;
};

/** CSS selector for any element that accepts a chip drop. Cards and the
 *  new-case zone both render this attribute so hit-testing is a single
 *  `closest(...)` walk in the pointermove handler. */
const CLASSIFY_DROP_TARGET_ATTR = "data-classify-drop-target";

export function ClassifyDropDialog({
  workspace,
  initialProposal,
  loading,
  onClose,
  onCommitted,
  onOpenCase,
  onError,
  onGoToSettings,
}: {
  workspace: Workspace;
  initialProposal: BatchCaseInput[];
  loading: boolean;
  onClose: () => void;
  onCommitted: () => void;
  onOpenCase: (caseId: string) => void;
  /** Bubble errors up to CasesPage so they survive the modal closing
   *  (the fire-and-forget `runAll` IPC may fail seconds after dismissal). */
  onError?: (msg: string) => void;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<BatchCaseInput[]>(initialProposal);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drag, setDrag] = useState<ClassifyDragState | null>(null);
  // Mirror of `drag` for use inside the window-level pointer listeners. The
  // listeners are installed once per drag and need to read the latest state
  // without re-binding on every cursor move.
  const dragRef = useRef<ClassifyDragState | null>(null);
  const updateDrag = useCallback(
    (
      next:
        | ClassifyDragState
        | null
        | ((prev: ClassifyDragState | null) => ClassifyDragState | null),
    ) => {
      const resolved =
        typeof next === "function" ? next(dragRef.current) : next;
      dragRef.current = resolved;
      setDrag(resolved);
    },
    [],
  );
  const [openNoteIdx, setOpenNoteIdx] = useState<number | null>(null);
  // Deliberative toggle for the batch path. When ON, every case in the
  // batch runs through the 4-pass committee. Drafts ignore the toggle
  // (they're persisted-only) — promoting a draft later uses quick mode.
  // Defaults to ON to match the single-case form; switch to Fast for a
  // bulk-triage pass.
  const [deliberative, setDeliberative] = useState(true);

  // Sync incoming proposal once it resolves from the loading state.
  useEffect(() => {
    setRows(initialProposal);
  }, [initialProposal]);

  useEffect(() => {
    (async () => {
      const ps = await ipc.listProviders();
      setProviders(ps);
      const eligible = usableProviders(ps).filter((p) => isClinicalEligible(p.id));
      const pick = preferredProvider(eligible);
      if (pick) {
        setProviderId(pick);
      } else if (eligible.length > 0) {
        // preferredProvider returned null on a non-empty list — fall back
        // so the run button is never silently disabled.
        setProviderId(eligible[0].id);
      } else if (ps.length > 0) {
        // No eligible provider but something is configured. Pick anyway;
        // backend `ensure_general_scope`/`ensure_provider_ready` will
        // give one clean error if the choice doesn't work clinically.
        setProviderId(ps[0].id);
      }
    })();
  }, []);

  // Mirror NewCase's clinical-eligibility filter: subtask-only providers
  // (Apple Intelligence today) must not be selectable for batch runs.
  // Without this filter the user could pick a Subtask provider, get a
  // backend rejection ("Provider scope is Subtask…") and have no idea
  // why the run failed.
  const usable = useMemo(
    () => usableProviders(providers).filter((p) => isClinicalEligible(p.id)),
    [providers],
  );

  const updateRow = (i: number, patch: Partial<BatchCaseInput>) => {
    setRows((prev) => {
      const next = [...prev];
      next[i] = { ...next[i], ...patch };
      return next;
    });
  };

  const removeRow = (i: number) => {
    setRows((prev) => {
      const row = prev[i];
      // Defensive: ask before removing a card that still has files —
      // those are about to be excluded from the batch.
      if (row && row.attached_file_paths.length > 0) {
        const ok = window.confirm(
          t("cases.classify_dialog_remove_case_confirm", {
            count: row.attached_file_paths.length,
          }),
        );
        if (!ok) return prev;
      }
      return prev.filter((_, idx) => idx !== i);
    });
  };

  /** Append an empty patient card so the user can create a case from
   *  scratch (e.g. typed notes with no attachments) without arranging
   *  a drag. Counterpart of `removeRow`. */
  const addEmptyCase = () => {
    setRows((prev) => [
      ...prev,
      {
        patient_label: t("cases.classify_dialog_new_case_default_label", {
          n: prev.length + 1,
        }),
        text: "",
        question: prev[0]?.question ?? t("cases.default_question"),
        attached_file_paths: [],
      },
    ]);
  };

  const removeFileFromRow = (rowIdx: number, fileIdx: number) => {
    setRows((prev) =>
      prev.map((r, idx) =>
        idx === rowIdx
          ? {
              ...r,
              attached_file_paths: r.attached_file_paths.filter(
                (_, i) => i !== fileIdx,
              ),
            }
          : r,
      ),
    );
  };

  const moveFile = (
    fromRow: number,
    fileIdx: number,
    targetRow: number | "new",
  ) => {
    setRows((prev) => {
      if (fromRow < 0 || fromRow >= prev.length) return prev;
      const file = prev[fromRow].attached_file_paths[fileIdx];
      if (file === undefined) return prev;
      const next = prev.map((r, idx) =>
        idx === fromRow
          ? {
              ...r,
              attached_file_paths: r.attached_file_paths.filter(
                (_, i) => i !== fileIdx,
              ),
            }
          : r,
      );
      if (targetRow === "new") {
        next.push({
          patient_label: deriveLabelFromFile(file, next.length + 1),
          text: "",
          question: prev[fromRow].question,
          attached_file_paths: [file],
        });
      } else if (targetRow >= 0 && targetRow < next.length) {
        next[targetRow] = {
          ...next[targetRow],
          attached_file_paths: [
            ...next[targetRow].attached_file_paths,
            file,
          ],
        };
      }
      return next.filter(
        (r) => r.attached_file_paths.length > 0 || r.text.trim().length > 0,
      );
    });
  };

  // Begin a chip drag. Called from the chip's `onPointerDown`. Installs
  // window-level pointer/keyboard listeners imperatively (rather than via a
  // useEffect keyed on `drag`) so we don't miss the first pointermove that
  // can fire before React commits the state change. The listeners read from
  // `dragRef.current` to stay current without re-binding.
  const beginChipDrag = (
    fromRow: number,
    fileIdx: number,
    startX: number,
    startY: number,
    filePath: string,
  ) => {
    if (busy) return;
    updateDrag({
      fromRow,
      fileIdx,
      filePath,
      startX,
      startY,
      cursorX: startX,
      cursorY: startY,
      activated: false,
      hoveringTarget: null,
    });

    const teardown = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKey, true);
    };

    const hitTest = (x: number, y: number): number | "new" | null => {
      const el = document
        .elementFromPoint(x, y)
        ?.closest(`[${CLASSIFY_DROP_TARGET_ATTR}]`);
      if (!el) return null;
      const raw = el.getAttribute(CLASSIFY_DROP_TARGET_ATTR);
      if (raw === "new") return "new";
      if (raw && raw.startsWith("card-")) {
        const idx = Number.parseInt(raw.slice("card-".length), 10);
        return Number.isFinite(idx) ? idx : null;
      }
      return null;
    };

    const onMove = (ev: PointerEvent) => {
      const current = dragRef.current;
      if (!current) {
        teardown();
        return;
      }
      const dx = ev.clientX - current.startX;
      const dy = ev.clientY - current.startY;
      const activated = current.activated || Math.hypot(dx, dy) > 5;
      const hoveringRaw = activated ? hitTest(ev.clientX, ev.clientY) : null;
      // Hide self-targeting: dropping back on the source row is a no-op,
      // so we never light it up as a hover target either.
      const hoveringTarget =
        hoveringRaw === current.fromRow ? null : hoveringRaw;
      updateDrag({
        ...current,
        cursorX: ev.clientX,
        cursorY: ev.clientY,
        activated,
        hoveringTarget,
      });
    };

    const onUp = (_ev: PointerEvent) => {
      teardown();
      const current = dragRef.current;
      updateDrag(null);
      if (!current || !current.activated) return;
      const target = current.hoveringTarget;
      if (target === null) return;
      if (target === current.fromRow) return;
      moveFile(current.fromRow, current.fileIdx, target);
    };

    const onCancel = (_ev: PointerEvent) => {
      teardown();
      updateDrag(null);
    };

    const onKey = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      if (!dragRef.current) return;
      // Swallow the keystroke so the dialog (and anything else above) sees
      // the drag-cancel rather than treating it as a request to close.
      ev.stopPropagation();
      ev.preventDefault();
      teardown();
      updateDrag(null);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    // Capture phase so we beat any document-level Escape handler that might
    // otherwise interpret the keystroke as "close the dialog".
    window.addEventListener("keydown", onKey, true);
  };

  const mergeAllIntoOne = () => {
    setRows((prev) => {
      if (prev.length <= 1) return prev;
      const allFiles = prev.flatMap((r) => r.attached_file_paths);
      const combinedText = prev
        .map((r) => r.text.trim())
        .filter(Boolean)
        .join("\n\n---\n\n");
      return [
        {
          patient_label: prev[0].patient_label,
          text: combinedText,
          question: prev[0].question,
          attached_file_paths: allFiles,
        },
      ];
    });
  };

  const splitEachFileIntoOwnCase = () => {
    setRows((prev) => {
      const split: BatchCaseInput[] = [];
      for (const r of prev) {
        if (r.attached_file_paths.length === 0) {
          split.push(r);
          continue;
        }
        r.attached_file_paths.forEach((file, i) => {
          split.push({
            patient_label:
              i === 0 && r.text.trim().length > 0
                ? r.patient_label
                : deriveLabelFromFile(file, split.length + 1),
            text: i === 0 ? r.text : "",
            question: r.question,
            attached_file_paths: [file],
          });
        });
      }
      return split;
    });
  };

  const saveAsDrafts = async () => {
    if (rows.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.createDraftCases({
        workspace_id: workspace.id,
        cases: rows,
      });
      onCommitted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const runAll = async () => {
    if (rows.length === 0) {
      setError(t("cases.classify_dialog_disabled_no_cases"));
      return;
    }
    if (!providerId) {
      // No provider available → don't leave the click without effect.
      // Persist as drafts so the cases land in the list, and surface a
      // banner explaining what happened. The user can then open each
      // draft and run it once they connect a provider in Settings.
      const diag = `providers=${providers.length}, usable=${usable.length}`;
      const fallback = `${t("cases.classify_dialog_disabled_no_provider")} ${t(
        "cases.classify_dialog_run_fallback_to_drafts",
      )} (${diag})`;
      onError?.(fallback);
      try {
        setBusy(true);
        await ipc.createDraftCases({
          workspace_id: workspace.id,
          cases: rows,
        });
        onCommitted();
      } catch (e) {
        // eslint-disable-next-line no-console
        console.error("fallback createDraftCases failed:", e);
        setError(String(e));
        onError?.(String(e));
      } finally {
        setBusy(false);
      }
      return;
    }
    // Fire-and-forget. The IPC awaits the whole batch to complete (5+
    // minutes for a deliberative run on 10 cases), so awaiting it here
    // would leave the user staring at a frozen dialog. Instead we
    // dispatch and close immediately — the page-level listeners for
    // `case:drafted` + `batch:progress` give the user real-time
    // feedback in the cases list.
    setError(null);
    void ipc
      .runBatchCases({
        workspace_id: workspace.id,
        provider_id: providerId,
        deliberative,
        cases: rows,
      })
      .catch((e) => {
        // eslint-disable-next-line no-console
        console.error("batch run failed:", e);
        // The dialog is gone by the time this resolves — surface the
        // error at the page level so the clinician sees it.
        onError?.(String(e));
      });
    onCommitted();
  };

  // Esc closes the dialog (unless we're mid-IPC, in which case the
  // user-visible "Cancel" footer button covers it). Mounted at the
  // window level so the keystroke catches even when the focus is
  // inside one of the patient cards' inputs.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  // Reason text for the run button's disabled state. Surfaced in a
  // `title` attribute so hovering tells the user exactly why nothing
  // happens when they click. Returns null when the button is enabled.
  const runDisabledReason = useMemo(() => {
    if (busy) return t("cases.classify_dialog_disabled_busy");
    if (rows.length === 0)
      return t("cases.classify_dialog_disabled_no_cases");
    if (!providerId)
      return t("cases.classify_dialog_disabled_no_provider");
    return null;
  }, [busy, rows.length, providerId, t]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("cases.classify_dialog_title")}
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/45 backdrop-blur-[2px] px-4 pb-4 pt-14"
    >
      <div className="flex max-h-[90vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-border bg-bg-elevated shadow-soft">
        <header className="flex items-start justify-between gap-3 border-b border-border-subtle px-5 py-4">
          <div className="min-w-0">
            <h2 className="text-[15px] font-semibold text-ink">
              {t("cases.classify_dialog_title")}
            </h2>
            <p className="mt-0.5 text-[12.5px] text-ink-subtle">
              {t("cases.classify_dialog_subtitle")}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label={t("cases.classify_dialog_close")}
            className="rounded p-1 text-ink-faint transition hover:bg-surface hover:text-ink"
          >
            <IconX size={16} stroke={1.6} aria-hidden />
          </button>
        </header>

        <div className="flex-1 space-y-4 overflow-y-auto px-5 py-4">
          {error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}

          {loading && (
            <div className="rounded-md border border-border-subtle bg-bg px-3 py-6 text-center text-[13px] text-ink-faint">
              {t("cases.classify_dialog_loading_proposal")}
            </div>
          )}

          {!loading && (
            <>
              <div className="rounded-md border border-warn/40 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">
                {t("cases.classify_dialog_banner")}
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={mergeAllIntoOne}
                  disabled={busy || rows.length <= 1}
                >
                  {t("cases.classify_dialog_merge_all")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={splitEachFileIntoOwnCase}
                  disabled={
                    busy ||
                    rows.every((r) => r.attached_file_paths.length <= 1)
                  }
                >
                  {t("cases.classify_dialog_split_all")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={addEmptyCase}
                  disabled={busy}
                >
                  {t("cases.classify_dialog_add_case")}
                </Button>
              </div>
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                {rows.map((row, i) => (
                  <ClassifyCard
                    key={`${row.patient_label}-${i}`}
                    row={row}
                    index={i}
                    isDragSource={!!drag?.activated && drag.fromRow === i}
                    isDropEligible={!!drag?.activated && drag.fromRow !== i}
                    isHoverTarget={drag?.hoveringTarget === i}
                    busy={busy}
                    noteOpen={openNoteIdx === i}
                    onToggleNote={() =>
                      setOpenNoteIdx(openNoteIdx === i ? null : i)
                    }
                    onLabelChange={(v) => updateRow(i, { patient_label: v })}
                    onQuestionChange={(v) => updateRow(i, { question: v })}
                    onTextChange={(v) => updateRow(i, { text: v })}
                    onRemoveCase={() => removeRow(i)}
                    onRemoveFile={(idx) => removeFileFromRow(i, idx)}
                    onChipPointerDown={(fileIdx, x, y, path) =>
                      beginChipDrag(i, fileIdx, x, y, path)
                    }
                  />
                ))}
                {drag?.activated && (
                  <ClassifyNewCardDropTarget
                    hover={drag.hoveringTarget === "new"}
                  />
                )}
              </div>
            </>
          )}
        </div>

        <footer className="space-y-3 border-t border-border-subtle px-5 py-4">
          <ProviderField
            providers={usable}
            providerId={providerId}
            onChange={setProviderId}
            onGoToSettings={onGoToSettings}
          />
          <ProviderOfflineBanner providers={providers} providerId={providerId} />
          <ModeSelector
            checked={deliberative}
            onChange={setDeliberative}
          />
          <div className="flex flex-wrap items-center justify-end gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={onClose}
              disabled={busy}
            >
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={saveAsDrafts}
              loading={busy}
              disabled={busy || rows.length === 0}
            >
              {t("cases.classify_dialog_save_drafts")}
            </Button>
            <span title={runDisabledReason ?? ""} className="inline-block">
              <Button
                size="sm"
                variant="primary"
                onClick={runAll}
                loading={busy}
                disabled={busy}
              >
                <span className="inline-flex items-center gap-1.5">
                  {t("cases.classify_dialog_run_all", { count: rows.length })}
                  {runDisabledReason && (
                    <IconAlertTriangle size={13} stroke={1.7} aria-hidden />
                  )}
                </span>
              </Button>
            </span>
          </div>
        </footer>
      </div>
      {drag?.activated && (
        <ClassifyDragGhost
          path={drag.filePath}
          x={drag.cursorX}
          y={drag.cursorY}
        />
      )}
      {/* Touch onOpenCase to silence the unused-warning lint until we
          surface a per-card "Open" affordance in a follow-up. */}
      {false && <span onClick={() => onOpenCase("")} />}
    </div>
  );
}

function ClassifyCard({
  row,
  index,
  isDragSource,
  isDropEligible,
  isHoverTarget,
  busy,
  noteOpen,
  onToggleNote,
  onLabelChange,
  onQuestionChange,
  onTextChange,
  onRemoveCase,
  onRemoveFile,
  onChipPointerDown,
}: {
  row: BatchCaseInput;
  index: number;
  /** True while one of *this* card's chips is being dragged. Dims the card
   *  so the user can see they're moving the chip away from it. */
  isDragSource: boolean;
  /** True while a sibling card's chip is in flight. Subtly highlights this
   *  card as a valid drop target without forcing the user to hover. */
  isDropEligible: boolean;
  /** True when the dragging cursor is currently over this card. The
   *  authoritative hit-test lives in the dialog's pointermove handler. */
  isHoverTarget: boolean;
  busy: boolean;
  noteOpen: boolean;
  onToggleNote: () => void;
  onLabelChange: (v: string) => void;
  onQuestionChange: (v: string) => void;
  onTextChange: (v: string) => void;
  onRemoveCase: () => void;
  onRemoveFile: (fileIdx: number) => void;
  /** Start a chip drag. `(x, y)` are the pointer coordinates at pointerdown,
   *  passed in so the dialog can derive the 5 px activation threshold. */
  onChipPointerDown: (
    fileIdx: number,
    x: number,
    y: number,
    path: string,
  ) => void;
}) {
  const { t } = useTranslation();

  return (
    <div
      data-classify-drop-target={`card-${index}`}
      className={cn(
        "rounded-lg border bg-bg p-3 transition",
        isHoverTarget
          ? "border-accent bg-accent/5 ring-1 ring-accent"
          : isDragSource
            ? "border-border-subtle opacity-70"
            : isDropEligible
              ? "border-accent/30 bg-accent/[0.02]"
              : "border-border-subtle",
      )}
    >
      <div className="flex items-center gap-2">
        <span className="font-mono text-[11px] text-ink-faint">
          {index + 1}
        </span>
        <input
          aria-label={t("cases.classify_dialog_patient_label")}
          className="flex-1 rounded border border-border-subtle bg-bg px-2 py-1 text-[13px] font-medium text-ink focus:border-accent focus:outline-none"
          value={row.patient_label}
          onChange={(e) => onLabelChange(e.target.value)}
          disabled={busy}
        />
        {!busy && (
          <button
            type="button"
            onClick={onRemoveCase}
            aria-label={t("cases.classify_dialog_remove_case")}
            title={t("cases.classify_dialog_remove_case")}
            className="inline-flex shrink-0 items-center gap-1 rounded-md border border-transparent px-2 py-1 text-[11.5px] font-medium text-danger/80 transition hover:border-danger/30 hover:bg-danger/10 hover:text-danger"
          >
            <IconX size={12} stroke={1.8} aria-hidden />
            <span>{t("cases.classify_dialog_remove_case")}</span>
          </button>
        )}
      </div>
      <input
        aria-label={t("cases.classify_dialog_question_label")}
        className="mt-2 w-full rounded border border-border-subtle bg-bg px-2 py-1 text-[12.5px] text-ink-dim focus:border-accent focus:outline-none"
        value={row.question}
        onChange={(e) => onQuestionChange(e.target.value)}
        placeholder={t("cases.classify_dialog_question_label")}
        disabled={busy}
      />
      <button
        type="button"
        onClick={onToggleNote}
        className="mt-2 rounded px-1 py-0.5 text-left text-[11.5px] text-ink-faint transition hover:text-ink"
        disabled={busy}
      >
        {noteOpen
          ? t("cases.classify_dialog_hide_note")
          : t("cases.classify_dialog_add_note")}
      </button>
      {noteOpen && (
        <Textarea
          value={row.text}
          onChange={(e) => onTextChange(e.target.value)}
          rows={3}
          placeholder={t("cases.field_text_placeholder")}
          disabled={busy}
        />
      )}
      <ul className="mt-3 flex flex-wrap gap-1.5">
        {row.attached_file_paths.length === 0 && (
          <li className="text-[11.5px] text-ink-faint">
            {t("cases.classify_dialog_no_files")}
          </li>
        )}
        {row.attached_file_paths.map((path, fileIdx) => (
          <li
            key={`${path}-${fileIdx}`}
            onPointerDown={(e) => {
              if (busy || e.button !== 0) return;
              // Don't start a drag when the press lands on the inline
              // "Quitar" button — let the click pass through unchanged.
              if (
                e.target instanceof Element &&
                e.target.closest("button[data-classify-chip-remove]")
              ) {
                return;
              }
              onChipPointerDown(fileIdx, e.clientX, e.clientY, path);
            }}
            style={{ touchAction: "none", userSelect: "none" }}
            className={cn(
              "group/chip flex max-w-full items-center gap-1.5 rounded-md border border-border-subtle bg-bg-subtle px-2 py-1 text-[11.5px] text-ink-dim transition",
              !busy && "cursor-grab hover:border-accent/40 hover:bg-bg hover:shadow-sm active:cursor-grabbing",
            )}
            title={busy ? path : t("cases.classify_dialog_chip_drag_hint", { path })}
          >
            <IconGripVertical
              size={12}
              stroke={1.6}
              aria-hidden
              className="select-none text-ink-faint group-hover/chip:text-accent"
            />
            <ClassifyFileChip path={path} />
            {!busy && (
              <button
                type="button"
                data-classify-chip-remove
                onClick={() => onRemoveFile(fileIdx)}
                aria-label={t("cases.attachment_remove")}
                className="rounded p-0.5 text-ink-faint transition hover:bg-surface hover:text-ink"
              >
                <IconX size={12} stroke={1.7} aria-hidden />
              </button>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function ClassifyFileChip({ path }: { path: string }) {
  const name = path.split(/[\\/]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  const ext = dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
  const isImage = ["png", "jpg", "jpeg", "webp", "tif", "tiff", "heic", "heif"].includes(
    ext,
  );
  return (
    <span className="flex min-w-0 items-center gap-1">
      <span
        className={cn(
          "shrink-0 rounded px-1 py-0.5 font-mono text-[9.5px] uppercase",
          attachmentBadgeColor(isImage ? "image" : ext),
        )}
      >
        {isImage ? "img" : ext || "?"}
      </span>
      <span className="truncate">{name}</span>
    </span>
  );
}

/** Pure presentational drop zone. Mounts only while a drag is active
 *  (the dialog gates on `drag?.activated`). The dialog's pointermove
 *  handler does the hit-testing and flips `hover` on; we just paint. */
function ClassifyNewCardDropTarget({ hover }: { hover: boolean }) {
  const { t } = useTranslation();
  return (
    <div
      data-classify-drop-target="new"
      className={cn(
        "flex items-center justify-center rounded-lg border border-dashed py-6 text-[12.5px] transition",
        hover
          ? "border-accent bg-accent/5 text-accent"
          : "border-border-subtle text-ink-faint",
      )}
    >
      + {t("cases.classify_dialog_drop_new_card")}
    </div>
  );
}

/** Floating chip preview that follows the cursor during a drag. Rendered
 *  via a portal into `document.body` so it sits above the dialog's z-40
 *  backdrop, and made `pointer-events: none` so it doesn't shadow the
 *  drop targets from `document.elementFromPoint`. */
function ClassifyDragGhost({
  path,
  x,
  y,
}: {
  path: string;
  x: number;
  y: number;
}) {
  return createPortal(
    <div
      aria-hidden
      style={{
        position: "fixed",
        left: 0,
        top: 0,
        transform: `translate(${x + 12}px, ${y + 12}px)`,
        pointerEvents: "none",
        zIndex: 50,
      }}
      className="flex max-w-[260px] items-center gap-1.5 rounded-md border border-accent bg-bg-elevated px-2 py-1 text-[11.5px] text-ink shadow-soft"
    >
      <IconGripVertical
        size={12}
        stroke={1.6}
        aria-hidden
        className="text-accent"
      />
      <ClassifyFileChip path={path} />
    </div>,
    document.body,
  );
}

function deriveLabelFromFile(path: string, fallbackIndex: number): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const stem = name.replace(/\.[^.]+$/, "").trim();
  return stem || `Paciente ${fallbackIndex}`;
}

// ---------------------------------------------------------------------------
// Deliberative mode — segmented Deliberate/Fast selector, in-flight overlay,
// post-hoc trace accordion.
// ---------------------------------------------------------------------------

// `checked` is the boolean carried through to the run command: true means
// Deliberate (4-pass committee, `run_case_deliberated`), false means Fast
// (single LLM call, `run_case`). Both options are first-class — the
// segmented control replaces the old "Deliberative mode" on/off toggle.
