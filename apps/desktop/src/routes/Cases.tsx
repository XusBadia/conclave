import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  IconAdjustments,
  IconFileTypePdf,
  IconRefresh,
  IconTrash,
  IconX,
} from "@tabler/icons-react";

import {
  exportCasesToFolder,
  type BatchExportResult,
} from "../pdf/exportCasesBatch";
import { loadPdfExportOptions } from "../pdf/exportOptions";
import { PdfOptionsSheet } from "../pdf/PdfOptionsSheet";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type BatchCaseInput,
  type CaseDetail,
  type CaseRecord,
  type Workspace,
} from "../lib/ipc";
import { isClinicalEligible, preferredProvider } from "../lib/providers";
import {
  attachmentFromPath,
  bucketKey,
  bucketLabel,
  dedupeAttachments,
  isFallbackLabel,
  localInputToIso,
  type GroupBy,
  type PendingAttachment,
  type SortBy,
} from "./cases/helpers";
import {
  BatchProgressBanner,
  PdfExportBanner,
  PdfExportResultBanner,
  PhaseRunningChip,
  StatusBadge,
} from "./cases/banners";
import {
  ConfirmDeletePopover,
  EditDateSheet,
} from "./cases/dialogs";
import { ClassifyDropDialog } from "./cases/ClassifyDropDialog";
import { NewCase } from "./cases/NewCase";
import { ShowCase } from "./cases/ShowCase";
import { useBatchProgress } from "./cases/useBatchProgress";

/** Ids the page is currently retrying so we don't fire duplicate calls
 *  on every refresh. Lives at module scope (not per-CasesPage) so the
 *  set survives unmounts while still scoped to the lifetime of the
 *  module (i.e., the app session). */
const labelRetryInflight = new Set<string>();

/** Best-effort retry of stale labels. Throttled to 2 concurrent IPCs
 *  because Apple Intelligence serialises calls on-device anyway and we
 *  don't want to block the user's clicks. Each retry that succeeds
 *  triggers a `case:drafted` event, which the page-level listener
 *  consumes to refresh the list. */
function retryStaleLabels(workspaceId: string, cases: CaseRecord[]): void {
  const candidates = cases.filter(
    (c) => isFallbackLabel(c.patient_label) && !labelRetryInflight.has(c.id),
  );
  // Cap concurrent retries — fire the first N, the rest will be picked
  // up by the NEXT refresh (which fires when the first ones complete
  // and emit `case:drafted`).
  const MAX_IN_FLIGHT = 2;
  const available = MAX_IN_FLIGHT - labelRetryInflight.size;
  for (const c of candidates.slice(0, Math.max(available, 0))) {
    labelRetryInflight.add(c.id);
    void ipc
      .regenerateCaseLabel(workspaceId, c.id)
      .finally(() => labelRetryInflight.delete(c.id));
  }
}

type View = "list" | "new" | "show";

export function CasesPage({
  workspace,
  onGoToSettings,
}: {
  workspace: Workspace;
  onGoToSettings?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [view, setView] = useState<View>("list");
  const [cases, setCases] = useState<CaseRecord[]>([]);
  const [selected, setSelected] = useState<CaseDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Files captured by the page-level drag-drop listener. When the user
  // drops onto the list view we route them to NewCase so they show up as
  // attachments right away; when the user is already in NewCase we hand
  // them off the same way and NewCase merges them with its current state.
  const [pendingDrop, setPendingDrop] = useState<PendingAttachment[]>([]);
  const [dropOverlay, setDropOverlay] = useState(false);
  /**
   * State of the classify-drop modal. Opened on multi-file drops; the
   * proposal is fetched from the backend heuristic so we render it with
   * editable patient cards rather than dumping everything into a single
   * NewCase form. `null` = modal closed.
   */
  const [classifyDialog, setClassifyDialog] = useState<{
    proposal: BatchCaseInput[];
    loading: boolean;
  } | null>(null);
  const [unsupportedDropError, setUnsupportedDropError] = useState<string | null>(
    null,
  );
  /**
   * Surface errors raised by the classify-drop dialog AFTER it closes —
   * `runAll` is fire-and-forget, so any IPC failure (provider offline,
   * missing config, …) can't be shown inside the modal. The page-level
   * banner lets the user see them without poking DevTools.
   */
  const [dialogError, setDialogError] = useState<string | null>(null);

  // Batch/live-progress state machine (see cases/useBatchProgress.ts).
  // The refresh callbacks go through refs because `refresh` is defined
  // below (it needs the hook's setRowBusy); the refs are reassigned on
  // every render so the hook always reaches the current closure.
  const refreshRef = useRef<() => void>(() => {});
  const scheduleRefreshRef = useRef<() => void>(() => {});
  const refreshNowRef = useRef<() => void>(() => {});
  const {
    runningCaseIds,
    batchTotal,
    batchDone,
    batchCancelling,
    batchStartedAtMs,
    batchFirstCaseMs,
    casePhases,
    rowBusy,
    setRowBusy,
    tickMs,
    cancelBatch,
  } = useBatchProgress({
    workspaceId: workspace.id,
    onRefresh: () => refreshRef.current(),
    onScheduleRefresh: () => scheduleRefreshRef.current(),
    onRefreshNow: () => refreshNowRef.current(),
  });

  // Sorting / grouping / selection. All client-side over the 50 rows that
  // listCases returns; the backend already sorts by case_date DESC so a
  // refresh keeps the natural order when sortBy === "date_desc".
  const [sortBy, setSortBy] = useState<SortBy>("date_desc");
  const [groupBy, setGroupBy] = useState<GroupBy>("off");
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [editingDate, setEditingDate] = useState(false);
  const [editDateError, setEditDateError] = useState<string | null>(null);
  const [editDateBusy, setEditDateBusy] = useState(false);
  // Multi-case PDF export. `pdfExporting` drives the progress banner;
  // `pdfExportSummary` the dismissible result banner once it finishes. The
  // abort controller lets the banner's Cancel stop the run between cases.
  const [pdfOptionsOpen, setPdfOptionsOpen] = useState(false);
  const [pdfExporting, setPdfExporting] = useState(false);
  const [pdfExportDone, setPdfExportDone] = useState(0);
  const [pdfExportTotal, setPdfExportTotal] = useState(0);
  const [pdfExportSummary, setPdfExportSummary] = useState<BatchExportResult | null>(null);
  const [pdfExportError, setPdfExportError] = useState<string | null>(null);
  const pdfAbortRef = useRef<AbortController | null>(null);
  // When non-null, the delete-confirmation popover is open for this
  // set of ids. Used both by the per-row hover trash button (length 1)
  // and by the batch toolbar (length N). `deleteAnchor` is the element
  // the popover anchors to (the actual button that was clicked) and
  // `deleteSource` flips the popover from below-the-trigger (rows) to
  // above-the-trigger (bulk toolbar at the bottom of the viewport).
  const [deletingIds, setDeletingIds] = useState<string[] | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deleteAnchor, setDeleteAnchor] = useState<HTMLElement | null>(null);
  const [deleteSource, setDeleteSource] = useState<"row" | "bulk" | null>(
    null,
  );

  /** Pending coalesced refresh (see `scheduleRefresh`). A batch cancel
   *  skips every queued case near-instantly; without coalescing we'd
   *  fire one `listCases` per skipped case. */
  const refreshTimerRef = useRef<number | null>(null);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await ipc.listCases(workspace.id, 50);
      setCases(list);
      // Clear `rowBusy` for any case that has settled to a terminal
      // status — keeps the "Cancelling…" / "Retrying…" chips honest
      // even when the batch event for that specific id never reaches
      // us (case_failed/case_cancelled don't carry case_id today).
      setRowBusy((prev) => {
        if (prev.size === 0) return prev;
        let changed = false;
        const next = new Map(prev);
        for (const c of list) {
          if (
            (c.status === "review_ready" ||
              c.status === "finalized" ||
              c.status === "finalized_legacy" ||
              c.status === "failed" ||
              c.status === "draft") &&
            next.has(c.id)
          ) {
            // If the case is back to Draft, the retry is being
            // dispatched — keep the "Retrying…" chip until a deliberation
            // event flips the row to running, OR a follow-up refresh
            // moves it to review_ready/finalized/failed.
            if (c.status === "draft" && next.get(c.id) === "retry") continue;
            next.delete(c.id);
            changed = true;
          }
        }
        return changed ? next : prev;
      });
      // Best-effort: re-run Apple Intelligence for cases whose label is
      // still the filename-stem fallback. The first attempt may have
      // failed (timeout, model not ready) or never run (CLI-imported
      // cases). The retry is fire-and-forget; the page refreshes via
      // the `case:drafted` listener whenever a label lands.
      retryStaleLabels(workspace.id, list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  /** Coalesce a burst of batch events into a single trailing refresh.
   *  Used by the per-case batch handlers; `batch_done` refreshes
   *  immediately (and cancels any pending timer). */
  const scheduleRefresh = () => {
    if (refreshTimerRef.current !== null) return; // already queued
    refreshTimerRef.current = window.setTimeout(() => {
      refreshTimerRef.current = null;
      void refresh();
    }, 120);
  };

  // Keep the useBatchProgress callbacks pointed at the live closures.
  refreshRef.current = () => void refresh();
  scheduleRefreshRef.current = scheduleRefresh;
  refreshNowRef.current = () => {
    if (refreshTimerRef.current !== null) {
      window.clearTimeout(refreshTimerRef.current);
      refreshTimerRef.current = null;
    }
    void refresh();
  };

  // Drop any pending coalesced refresh when the page unmounts.
  useEffect(() => {
    return () => {
      if (refreshTimerRef.current !== null) {
        window.clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    refresh();
    setView("list");
    setSelected(null);
    setSortBy("date_desc");
    setGroupBy("off");
    setSelectionMode(false);
    setSelectedIds(new Set());
    setEditingDate(false);
    setEditDateError(null);
    setDeletingIds(null);
    setDeleteError(null);
    setDeleteAnchor(null);
    setDeleteSource(null);
    setPendingDrop([]);
    setUnsupportedDropError(null);
    setClassifyDialog(null);
  }, [workspace.id]);

  // Page-level Tauri drag-drop listener. Bound once for the whole Cases
  // route so a clinician can drop PDFs / images anywhere on the cases
  // screen — list, new-case form, show-case — and have them attached to
  // the case they're composing. Existing per-view drop targets (the
  // dropzone inside NewCase) still work because this listener only
  // pushes the payload into `pendingDrop`; consumers pick it up via the
  // standard React state flow.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const webview = getCurrentWebviewWindow();
      const fn = await webview.onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDropOverlay(true);
        } else if (event.payload.type === "leave") {
          setDropOverlay(false);
        } else if (event.payload.type === "drop") {
          setDropOverlay(false);
          const paths = event.payload.paths;
          if (paths.length === 0) return;
          const accepted: PendingAttachment[] = [];
          const rejected: string[] = [];
          for (const p of paths) {
            const a = attachmentFromPath(p);
            if (a) accepted.push(a);
            else rejected.push(p);
          }
          if (accepted.length > 0) {
            setUnsupportedDropError(null);
            if (accepted.length === 1) {
              // Single file → NewCase, same as before.
              setPendingDrop((prev) => dedupeAttachments(prev, accepted));
              setView((v) => (v === "list" || v === "show" ? "new" : v));
            } else {
              // Multi-file → classify-drop modal. We ask the backend for
              // the heuristic proposal first; while it's in-flight the
              // modal renders a `loading` state.
              setClassifyDialog({ proposal: [], loading: true });
              (async () => {
                try {
                  const proposal = await ipc.proposeCaseGrouping(
                    accepted.map((a) => a.path),
                    t("cases.default_question"),
                  );
                  setClassifyDialog({ proposal, loading: false });
                } catch (e) {
                  setUnsupportedDropError(String(e));
                  setClassifyDialog(null);
                }
              })();
            }
          }
          if (rejected.length > 0 && accepted.length === 0) {
            setUnsupportedDropError(
              t("cases.attachment_unsupported", { count: rejected.length }),
            );
          }
        }
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [t]);


  const sortedCases = useMemo(() => {
    const arr = [...cases];
    switch (sortBy) {
      case "date_desc":
        return arr.sort((a, b) => b.case_date.localeCompare(a.case_date));
      case "date_asc":
        return arr.sort((a, b) => a.case_date.localeCompare(b.case_date));
      case "question_az":
        return arr.sort((a, b) =>
          (a.question || "").localeCompare(b.question || "", undefined, {
            sensitivity: "base",
          }),
        );
      case "status":
        return arr.sort((a, b) => a.status.localeCompare(b.status));
    }
  }, [cases, sortBy]);

  const groupsEnabled = groupBy !== "off" && sortBy.startsWith("date");
  const locale = i18n.language || "es";

  // Rows interleaved with group headers, in display order.
  type Row =
    | { kind: "header"; key: string; label: string }
    | { kind: "case"; key: string; data: CaseRecord };
  const rows = useMemo<Row[]>(() => {
    if (!groupsEnabled) {
      return sortedCases.map((c) => ({ kind: "case", key: c.id, data: c }));
    }
    const out: Row[] = [];
    let currentKey = "";
    for (const c of sortedCases) {
      const k = bucketKey(c.case_date, groupBy);
      if (k !== currentKey) {
        currentKey = k;
        out.push({
          kind: "header",
          key: `h-${k}`,
          label: bucketLabel(c.case_date, groupBy, t, locale),
        });
      }
      out.push({ kind: "case", key: c.id, data: c });
    }
    return out;
  }, [sortedCases, groupBy, groupsEnabled, t, locale]);

  const toggleSelected = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const exitSelection = () => {
    setSelectionMode(false);
    setSelectedIds(new Set());
  };

  // True when every loaded case is already selected. Drives the
  // "Select all" ↔ "Deselect all" toggle label. The `cases.length > 0`
  // guard stops an empty list from reporting "all selected".
  const allSelected =
    cases.length > 0 && cases.every((c) => selectedIds.has(c.id));

  /** Select every loaded case, or clear the selection if they're all
   *  already selected. Operates over `cases` — the up-to-50 rows the
   *  list actually shows — so it matches exactly what the user sees. */
  const toggleSelectAll = () => {
    setSelectedIds(allSelected ? new Set() : new Set(cases.map((c) => c.id)));
  };

  /** Export the selected cases to a folder, one PDF per case. Reads the
   *  persisted PDF options fresh at click time so a change made in the
   *  single-case view is honoured here too. Verdict-less cases are skipped
   *  and reported in the result banner. */
  const onBatchExportPdf = useCallback(async () => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0 || pdfExporting) return;
    const controller = new AbortController();
    pdfAbortRef.current = controller;
    setPdfExporting(true);
    setPdfExportError(null);
    setPdfExportSummary(null);
    setPdfExportDone(0);
    setPdfExportTotal(ids.length);
    try {
      const result = await exportCasesToFolder(
        workspace.id,
        ids,
        t,
        i18n.language,
        loadPdfExportOptions(),
        ({ done, total }) => {
          setPdfExportDone(done);
          setPdfExportTotal(total);
        },
        controller.signal,
      );
      // A dismissed folder picker leaves nothing to report. Anything else
      // (full run, partial abort) gets a summary banner.
      if (!result.cancelled) {
        setPdfExportSummary(result);
        exitSelection();
      }
    } catch (e) {
      setPdfExportError(String(e));
    } finally {
      setPdfExporting(false);
      pdfAbortRef.current = null;
    }
  }, [selectedIds, pdfExporting, workspace.id, t, i18n.language]);

  const onCancelPdfExport = useCallback(() => {
    pdfAbortRef.current?.abort();
  }, []);

  const onApplyDate = async (localValue: string) => {
    if (selectedIds.size === 0) return;
    setEditDateBusy(true);
    setEditDateError(null);
    try {
      const iso = localInputToIso(localValue);
      await ipc.updateCaseDate({
        workspace_id: workspace.id,
        case_ids: Array.from(selectedIds),
        new_date: iso,
      });
      setEditingDate(false);
      exitSelection();
      await refresh();
    } catch (e) {
      setEditDateError(String(e));
    } finally {
      setEditDateBusy(false);
    }
  };

  // Pick the initial date for the picker — the case_date of the first
  // selected case, or now if the selection is empty for any reason.
  const initialEditIso = useMemo(() => {
    const firstId = Array.from(selectedIds)[0];
    const found = firstId ? cases.find((c) => c.id === firstId) : null;
    return found?.case_date ?? new Date().toISOString();
  }, [selectedIds, cases]);

  /** Optimistic open-case: flip the view immediately with `selected =
   *  null` so ShowCase renders its skeleton, then load the detail in
   *  the background and patch state when it arrives. Replaces the
   *  blank 1–2 s pause we had on completed-case clicks. */
  const openCaseOptimistic = useCallback(
    async (record: CaseRecord) => {
      // Drafts route to NewCase pre-filled; everything else to ShowCase.
      if (record.status === "draft") {
        setSelected(null);
        const det = await ipc.showCase(workspace.id, record.id);
        setSelected(det);
        setView("new");
        return;
      }
      setSelected(null);
      setView("show");
      try {
        const det = await ipc.showCase(workspace.id, record.id);
        setSelected(det);
      } catch {
        // showCase returned an error — go back to the list rather than
        // leaving the user on an infinite-skeleton screen.
        setView("list");
      }
    },
    [workspace.id],
  );

  /** Cancel the in-flight LLM call for a single case row. Backend
   *  flips the per-case AtomicBool; the deliberation pipeline checks
   *  it between phases and bails out. */
  const onCancelRow = useCallback(async (caseId: string) => {
    setRowBusy((prev) => {
      const next = new Map(prev);
      next.set(caseId, "cancel");
      return next;
    });
    try {
      await ipc.cancelCase(caseId);
    } catch {
      // Swallow: the backend returns Ok even for unknown ids, so any
      // hard error here is exotic. We let the batch listener clear
      // the row once the case settles to Failed/Cancelled.
    } finally {
      // Don't clear `rowBusy` immediately — the row stays in
      // "cancelling…" until the batch event lands. We clear it from
      // the listener via `clearRowBusy` (see below) on the next
      // refresh.
    }
  }, [workspace.id]);

  /** Cancel the entire batch — see useBatchProgress.cancelBatch. */
  const onCancelAll = cancelBatch;

  /** Reset a failed row to draft and re-run via the current provider. */
  const onRetryRow = useCallback(
    async (caseId: string) => {
      // Need a provider — pick the first usable / clinical-eligible.
      const ps = await ipc.listProviders();
      const eligible = usableProviders(ps).filter((p) =>
        isClinicalEligible(p.id),
      );
      const pick = preferredProvider(eligible) ?? eligible[0]?.id;
      if (!pick) {
        setError(t("cases.no_provider_configured"));
        return;
      }
      setRowBusy((prev) => {
        const next = new Map(prev);
        next.set(caseId, "retry");
        return next;
      });
      try {
        await ipc.resetCaseToDraft(workspace.id, caseId);
        // Reuse runDraftCase — it re-runs the (now Draft) case with the
        // existing text/attachments. Fire-and-forget; case_drafted +
        // batch events will refresh the row.
        void ipc
          .runDraftCase({
            workspace_id: workspace.id,
            case_id: caseId,
            provider_id: pick,
          })
          .catch((e) => {
            setError(String(e));
          })
          .finally(() => {
            setRowBusy((prev) => {
              if (!prev.has(caseId)) return prev;
              const next = new Map(prev);
              next.delete(caseId);
              return next;
            });
            void refresh();
          });
      } catch (e) {
        setError(String(e));
        setRowBusy((prev) => {
          if (!prev.has(caseId)) return prev;
          const next = new Map(prev);
          next.delete(caseId);
          return next;
        });
      }
    },
    // refresh is captured from the outer scope and never changes
    // identity in a way the list cares about.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [workspace.id, t],
  );

  const onConfirmDelete = async () => {
    if (!deletingIds || deletingIds.length === 0) return;
    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await ipc.deleteCases({
        workspace_id: workspace.id,
        case_ids: deletingIds,
      });
      // If the case currently being shown was deleted, drop the
      // viewer state so the user doesn't see a stale verdict.
      if (selected && deletingIds.includes(selected.case.id)) {
        setSelected(null);
        setView("list");
      }
      // Drop the deleted ids from the multi-select set so the batch
      // toolbar collapses naturally.
      setSelectedIds((prev) => {
        const next = new Set(prev);
        for (const id of deletingIds) next.delete(id);
        return next;
      });
      setDeletingIds(null);
      // If selection mode is on but nothing is selected anymore,
      // exit it so the row UI returns to its normal state.
      if (selectionMode && selectedIds.size <= deletingIds.length) {
        setSelectionMode(false);
      }
      await refresh();
    } catch (e) {
      setDeleteError(String(e));
    } finally {
      setDeleteBusy(false);
    }
  };

  if (view === "new") {
    // When `selected` is a draft (clicked from the list), we hand it to
    // NewCase so it pre-fills text / question / attachments and runs via
    // `runDraftCase` against the existing case id.
    const draft =
      selected && selected.case.status === "draft" ? selected : null;
    return (
      <NewCase
        workspace={workspace}
        onCancel={() => {
          setView("list");
          setPendingDrop([]);
          setSelected(null);
        }}
        onGoToSettings={onGoToSettings}
        incomingAttachments={pendingDrop}
        onIncomingConsumed={() => setPendingDrop([])}
        draft={draft}
        onDone={async (id) => {
          setPendingDrop([]);
          setSelected(null);
          await refresh();
          const det = await ipc.showCase(workspace.id, id);
          setSelected(det);
          setView("show");
        }}
      />
    );
  }

  if (view === "show") {
    // `selected` may be null during the optimistic open — ShowCase
    // renders its own skeleton until the detail lands.
    return (
      <ShowCase
        workspace={workspace}
        detail={selected}
        onBack={() => {
          setSelected(null);
          setView("list");
        }}
      />
    );
  }

  return (
    <div className="relative mx-auto w-full max-w-5xl space-y-4 p-6 pb-24">
      {dropOverlay && (
        <div
          aria-hidden
          className="pointer-events-none fixed inset-0 z-30 flex items-center justify-center bg-accent/15 backdrop-blur-[2px]"
        >
          <div className="rounded-2xl border-2 border-dashed border-accent bg-bg-elevated/90 px-8 py-6 text-center shadow-soft">
            <p className="text-[14px] font-semibold text-ink">
              {t("cases.drop_overlay_title")}
            </p>
            <p className="mt-1 text-[12px] text-ink-subtle">
              {t("cases.drop_overlay_hint")}
            </p>
          </div>
        </div>
      )}
      {unsupportedDropError && (
        <div className="rounded-md border border-warn/40 bg-warn/10 px-3 py-2 text-[13px] text-warn">
          {unsupportedDropError}
        </div>
      )}
      {dialogError && (
        <div className="flex items-start justify-between gap-3 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
          <span className="break-words">{dialogError}</span>
          <button
            type="button"
            onClick={() => setDialogError(null)}
            aria-label={t("common.dismiss")}
            className="shrink-0 rounded p-0.5 text-danger/70 transition hover:bg-danger/10 hover:text-danger"
          >
            <IconX size={14} stroke={1.7} aria-hidden />
          </button>
        </div>
      )}
      {batchTotal !== null && batchTotal > 0 && (
        <BatchProgressBanner
          done={batchDone}
          total={batchTotal}
          startedAtMs={batchStartedAtMs}
          firstCaseMs={batchFirstCaseMs}
          tickMs={tickMs}
          cancelling={batchCancelling}
          onCancelAll={onCancelAll}
        />
      )}
      {pdfExporting && (
        <PdfExportBanner
          done={pdfExportDone}
          total={pdfExportTotal}
          onCancel={onCancelPdfExport}
        />
      )}
      {pdfExportSummary && !pdfExporting && (
        <PdfExportResultBanner
          result={pdfExportSummary}
          onDismiss={() => setPdfExportSummary(null)}
        />
      )}
      {pdfExportError && (
        <div className="flex items-start justify-between gap-3 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
          <span className="break-words">
            {t("cases.batch_export_error", { error: pdfExportError })}
          </span>
          <button
            type="button"
            onClick={() => setPdfExportError(null)}
            aria-label={t("common.dismiss")}
            className="shrink-0 rounded p-0.5 text-danger/70 transition hover:bg-danger/10 hover:text-danger"
          >
            <IconX size={14} stroke={1.7} aria-hidden />
          </button>
        </div>
      )}
      <Card>
        <CardHeader
          title={t("cases.page_title")}
          subtitle={t("cases.page_subtitle", {
            count: cases.length,
            workspace: workspace.name,
          })}
          right={
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={refresh} loading={loading}>
                {t("common.refresh")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  setUnsupportedDropError(null);
                  try {
                    const picked = await openDialog({
                      multiple: false,
                      directory: true,
                      title: t("cases.process_folder_pick_title"),
                    });
                    if (!picked) return;
                    setClassifyDialog({ proposal: [], loading: true });
                    const proposal = await ipc.parseBatchFolder(
                      String(picked),
                      t("cases.default_question"),
                    );
                    if (proposal.length === 0) {
                      setClassifyDialog(null);
                      setUnsupportedDropError(
                        t("cases.process_folder_empty"),
                      );
                      return;
                    }
                    setClassifyDialog({ proposal, loading: false });
                  } catch (e) {
                    setClassifyDialog(null);
                    setUnsupportedDropError(String(e));
                  }
                }}
              >
                {t("cases.process_folder_button")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                onClick={() => setView("new")}
              >
                {t("cases.new_button")}
              </Button>
            </div>
          }
        />
        <CardBody className="p-0">
          {error && (
            <div className="mx-5 mt-4 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          {cases.length > 0 && (
            <div className="flex flex-wrap items-center gap-3 border-b border-border-subtle px-5 py-2.5 text-[12px]">
              <label className="flex items-center gap-1.5 text-ink-subtle">
                <span>{t("cases.sort_by")}</span>
                <select
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value as SortBy)}
                  className="rounded-md border border-border bg-bg px-2 py-1 text-ink focus:outline-none focus:ring-conclave"
                >
                  <option value="date_desc">{t("cases.sort.date_desc")}</option>
                  <option value="date_asc">{t("cases.sort.date_asc")}</option>
                  <option value="question_az">{t("cases.sort.question_az")}</option>
                  <option value="status">{t("cases.sort.status")}</option>
                </select>
              </label>
              {sortBy.startsWith("date") && (
                <label className="flex items-center gap-1.5 text-ink-subtle">
                  <span>{t("cases.group_by")}</span>
                  <select
                    value={groupBy}
                    onChange={(e) => setGroupBy(e.target.value as GroupBy)}
                    className="rounded-md border border-border bg-bg px-2 py-1 text-ink focus:outline-none focus:ring-conclave"
                  >
                    <option value="off">{t("cases.group.off")}</option>
                    <option value="day">{t("cases.group.day")}</option>
                    <option value="week">{t("cases.group.week")}</option>
                    <option value="month">{t("cases.group.month")}</option>
                  </select>
                </label>
              )}
              <div className="ml-auto flex items-center gap-2">
                {!selectionMode ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setSelectionMode(true)}
                  >
                    {t("cases.select")}
                  </Button>
                ) : (
                  <>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={toggleSelectAll}
                    >
                      {allSelected
                        ? t("cases.deselect_all")
                        : t("cases.select_all")}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={exitSelection}>
                      {t("cases.cancel_selection")}
                    </Button>
                  </>
                )}
              </div>
            </div>
          )}
          {cases.length === 0 && !loading && (
            <div className="px-6 py-12 text-center">
              <p className="text-[13px] text-ink-subtle">
                {t("cases.empty_title")}
              </p>
              <div className="mt-4">
                <Button variant="primary" onClick={() => setView("new")}>
                  {t("cases.new_button")}
                </Button>
              </div>
            </div>
          )}
          <ul className="divide-y divide-border-subtle">
            {rows.map((row) => {
              if (row.kind === "header") {
                return (
                  <li
                    key={row.key}
                    className="bg-bg-subtle px-5 py-1.5 text-[11px] font-medium uppercase tracking-wide text-ink-faint"
                  >
                    {row.label}
                  </li>
                );
              }
              const c = row.data;
              const isSelected = selectedIds.has(c.id);
              const phase = casePhases.get(c.id);
              const isRunning = runningCaseIds.has(c.id) || phase !== undefined;
              const rowAction = rowBusy.get(c.id);
              const openCase = () => {
                if (selectionMode) {
                  toggleSelected(c.id);
                  return;
                }
                void openCaseOptimistic(c);
              };
              return (
                // `group` on the <li> drives the per-row hover-only
                // delete button (opacity-0 → opacity-100 on group-hover).
                // The row is a div role="button" rather than a real
                // <button>, so we can nest a real <button> for delete
                // without producing invalid nested-button markup.
                <li key={row.key} className="group relative">
                  <div
                    role="button"
                    tabIndex={0}
                    onClick={openCase}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        openCase();
                      }
                    }}
                    className={cn(
                      "block w-full cursor-pointer px-5 py-4 text-left transition focus:outline-none focus-visible:bg-surface",
                      isSelected ? "bg-accent/5 hover:bg-accent/10" : "hover:bg-surface",
                    )}
                  >
                    <div className="flex items-center gap-3">
                      {selectionMode && (
                        <input
                          type="checkbox"
                          checked={isSelected}
                          readOnly
                          aria-label={c.patient_label || c.question || c.id}
                          className="h-4 w-4 shrink-0 accent-accent"
                          tabIndex={-1}
                        />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[14px] font-medium text-ink">
                          {c.patient_label || c.question || t("cases.no_question")}
                        </div>
                        <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[12px] text-ink-faint">
                          {c.patient_label && c.question ? (
                            <span className="truncate">{c.question}</span>
                          ) : null}
                          {c.patient_label && c.question ? <span>·</span> : null}
                          <span>{new Date(c.case_date).toLocaleString()}</span>
                          {phase && (
                            <PhaseRunningChip phase={phase} tickMs={tickMs} />
                          )}
                        </div>
                      </div>
                      <StatusBadge
                        status={c.status}
                        running={isRunning}
                      />
                      {!selectionMode && c.status === "failed" && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            void onRetryRow(c.id);
                          }}
                          disabled={rowAction === "retry"}
                          aria-label={t("cases.row_retry_failed")}
                          title={t("cases.row_retry_failed")}
                          className={cn(
                            "shrink-0 inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11.5px]",
                            "text-accent transition hover:bg-accent/10",
                            "focus:outline-none focus-visible:ring-conclave",
                            rowAction === "retry" && "opacity-60",
                          )}
                        >
                          <IconRefresh
                            aria-hidden="true"
                            size={14}
                            stroke={1.6}
                            className={rowAction === "retry" ? "animate-spin" : ""}
                          />
                          <span>
                            {rowAction === "retry"
                              ? t("cases.row_retry_busy")
                              : t("cases.row_retry_failed")}
                          </span>
                        </button>
                      )}
                      {!selectionMode && isRunning && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            void onCancelRow(c.id);
                          }}
                          disabled={rowAction === "cancel"}
                          aria-label={t("cases.row_cancel_running")}
                          title={t("cases.row_cancel_running")}
                          className={cn(
                            "shrink-0 inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11.5px]",
                            "text-ink-subtle transition hover:bg-danger/10 hover:text-danger",
                            "focus:outline-none focus-visible:ring-conclave",
                            rowAction === "cancel" && "opacity-60",
                          )}
                        >
                          <IconX
                            aria-hidden="true"
                            size={14}
                            stroke={1.6}
                          />
                          <span>
                            {rowAction === "cancel"
                              ? t("cases.row_cancel_busy")
                              : t("cases.row_cancel_running")}
                          </span>
                        </button>
                      )}
                      {!selectionMode && !isRunning && c.status !== "failed" && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeleteError(null);
                            setDeleteAnchor(e.currentTarget);
                            setDeleteSource("row");
                            setDeletingIds([c.id]);
                          }}
                          aria-label={t("cases.delete_row")}
                          title={t("cases.delete_row")}
                          // Collapsed by default (w-0 + -ml-3 cancels the
                          // parent gap-3 so the row content sits flush
                          // right). On row hover (or keyboard focus) the
                          // button grows to its natural size and the
                          // status badge slides left to make room — the
                          // shift is the affordance. While this row's
                          // delete popover is open the button is pinned
                          // expanded so the popover's anchor doesn't
                          // collapse out from under it.
                          className={cn(
                            "grid h-7 w-0 shrink-0 place-content-center overflow-hidden rounded-md",
                            "-ml-3 text-ink-faint opacity-0",
                            "transition-[width,margin,opacity,background-color,color] duration-200 ease-out",
                            "hover:bg-danger/10 hover:text-danger",
                            "group-hover:ml-0 group-hover:w-7 group-hover:opacity-100",
                            "focus:ml-0 focus:w-7 focus:opacity-100 focus:outline-none focus-visible:ring-conclave",
                            deleteSource === "row" &&
                              deletingIds?.length === 1 &&
                              deletingIds[0] === c.id &&
                              "ml-0 w-7 bg-danger/10 text-danger opacity-100",
                          )}
                        >
                          <IconTrash
                            aria-hidden="true"
                            size={16}
                            stroke={1.6}
                          />
                        </button>
                      )}
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        </CardBody>
      </Card>

      {selectionMode && selectedIds.size > 0 && (
        <div className="fixed inset-x-0 bottom-0 z-20 border-t border-border bg-bg-elevated/95 px-6 py-3 shadow-soft backdrop-blur">
          <div className="mx-auto flex max-w-5xl items-center justify-between gap-3">
            <span className="text-[13px] text-ink-dim">
              {t("cases.selected_count", { count: selectedIds.size })}
            </span>
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={exitSelection}>
                {t("common.cancel")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setPdfOptionsOpen(true)}
                aria-label={t("cases.export_options")}
                title={t("cases.export_options")}
              >
                <IconAdjustments size={15} stroke={1.6} />
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={onBatchExportPdf}
                loading={pdfExporting}
                leftIcon={<IconFileTypePdf size={14} stroke={1.6} />}
              >
                {t("cases.export_pdf_action")}
              </Button>
              <Button
                size="sm"
                variant="danger"
                onClick={(e) => {
                  setDeleteError(null);
                  setDeleteAnchor(e.currentTarget);
                  setDeleteSource("bulk");
                  setDeletingIds(Array.from(selectedIds));
                }}
              >
                {t("cases.delete_action")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  setEditDateError(null);
                  setEditingDate(true);
                }}
              >
                {t("cases.edit_date_action")}
              </Button>
            </div>
          </div>
        </div>
      )}

      <EditDateSheet
        open={editingDate}
        onOpenChange={(next) => {
          setEditingDate(next);
          if (!next) setEditDateError(null);
        }}
        count={selectedIds.size}
        initialIso={initialEditIso}
        busy={editDateBusy}
        error={editDateError}
        onApply={onApplyDate}
      />

      <PdfOptionsSheet open={pdfOptionsOpen} onOpenChange={setPdfOptionsOpen} />

      <ConfirmDeletePopover
        open={deletingIds !== null}
        onOpenChange={(next) => {
          if (!next) {
            setDeletingIds(null);
            setDeleteError(null);
            setDeleteAnchor(null);
            setDeleteSource(null);
          }
        }}
        anchor={deleteAnchor}
        side={deleteSource === "bulk" ? "top" : "bottom"}
        align="end"
        count={deletingIds?.length ?? 0}
        busy={deleteBusy}
        error={deleteError}
        onConfirm={onConfirmDelete}
      />

      {classifyDialog && (
        <ClassifyDropDialog
          workspace={workspace}
          initialProposal={classifyDialog.proposal}
          loading={classifyDialog.loading}
          onClose={() => setClassifyDialog(null)}
          onCommitted={async () => {
            setClassifyDialog(null);
            setDialogError(null);
            await refresh();
          }}
          onOpenCase={async (caseId) => {
            setClassifyDialog(null);
            const det = await ipc.showCase(workspace.id, caseId);
            setSelected(det);
            setView(det && det.case.status === "draft" ? "new" : "show");
          }}
          onError={(msg) => setDialogError(msg)}
          onGoToSettings={onGoToSettings}
        />
      )}
    </div>
  );
}
