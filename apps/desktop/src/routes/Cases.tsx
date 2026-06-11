import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  IconAdjustments,
  IconFileTypePdf,
  IconLock,
  IconRefresh,
  IconTrash,
  IconX,
} from "@tabler/icons-react";

import { exportCaseVerdictToPDF } from "../pdf/exportCaseVerdict";
import {
  exportCasesToFolder,
  type BatchExportResult,
} from "../pdf/exportCasesBatch";
import { loadPdfExportOptions } from "../pdf/exportOptions";
import { PdfOptionsSheet } from "../pdf/PdfOptionsSheet";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input, Textarea } from "../components/Field";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type BatchCaseInput,
  type CaseAttachment,
  type CaseDetail,
  type DataBoundaryMode,
  type DataBoundaryPreview,
  type CaseRecord,
  type ProviderInfo,
  type Skill,
  type Workspace,
} from "../lib/ipc";
import { isReady } from "../lib/providerStatus";
import { isClinicalEligible, metaFor, preferredProvider } from "../lib/providers";
import {
  attachmentBadgeColor,
  attachmentFromPath,
  bucketKey,
  bucketLabel,
  dedupeAttachments,
  formatBytes,
  isFallbackLabel,
  localInputToIso,
  type GroupBy,
  type PendingAttachment,
  type SortBy,
  SUPPORTED_ATTACHMENT_EXTS,
} from "./cases/helpers";
import {
  BatchProgressBanner,
  PdfExportBanner,
  PdfExportResultBanner,
  PhaseRunningChip,
  ModeSelector,
  ProviderOfflineBanner,
  ShowCaseSkeleton,
  StatusBadge,
} from "./cases/banners";
import {
  ConfirmDeletePopover,
  ConfirmPurgePopover,
  EditDateSheet,
} from "./cases/dialogs";
import { ClassifyDropDialog } from "./cases/ClassifyDropDialog";
import { ProviderField } from "./cases/ProviderField";
import { useBatchProgress } from "./cases/useBatchProgress";
import {
  DeliberationOverlay,
  DeliberationTraceAccordion,
  FailedCaseErrorBlock,
} from "./cases/DeliberationOverlay";
import { VerdictRenderer } from "./cases/VerdictRenderer";

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

function NewCase({
  workspace,
  onCancel,
  onDone,
  onGoToSettings,
  incomingAttachments,
  onIncomingConsumed,
  draft,
}: {
  workspace: Workspace;
  onCancel: () => void;
  onDone: (caseId: string) => void;
  onGoToSettings?: () => void;
  incomingAttachments?: PendingAttachment[];
  onIncomingConsumed?: () => void;
  /**
   * When set, NewCase boots in "edit draft" mode: pre-fills text /
   * question, loads the persisted attachments, and on submit calls
   * `runDraftCase` against the existing case id instead of minting a
   * fresh one via `runCase`.
   */
  draft?: CaseDetail | null;
}) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [skills, setSkills] = useState<Skill[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [text, setText] = useState(draft?.case.original_text ?? "");
  const [question, setQuestion] = useState(
    draft?.case.question ?? t("cases.default_question"),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dataBoundaryMode, setDataBoundaryMode] =
    useState<DataBoundaryMode>("deid_cloud");
  const [allowPhiPayload, setAllowPhiPayload] = useState(false);
  const [retainRawText, setRetainRawText] = useState(false);
  const [useOnlineEvidence, setUseOnlineEvidence] = useState(false);
  const [activeSkillId, setActiveSkillId] = useState("");
  const [boundaryPreview, setBoundaryPreview] =
    useState<DataBoundaryPreview | null>(null);
  const [attachments, setAttachments] = useState<PendingAttachment[]>(() =>
    incomingAttachments ?? [],
  );
  // Persisted attachments belonging to the draft. Rendered read-only —
  // the clinician can drop NEW files but not remove ones already saved
  // (that would require a separate delete command). Cleared once the
  // draft promotes.
  const [draftAttachments, setDraftAttachments] = useState<CaseAttachment[]>([]);
  /**
   * When ON, the run button calls `run_case_deliberated` instead of
   * `run_case` — the LLM does briefing → drafting → red-team → finalize,
   * costs more tokens, and the user sees a live 4-seat overlay while
   * the committee thinks. Defaults to ON; clinicians who want a quick
   * single-pass triage flip to Fast via the mode selector.
   */
  const [deliberative, setDeliberative] = useState(true);
  /**
   * Set while a deliberative case is in flight. Owns the overlay and
   * its event subscription. Cleared when the run resolves (either via
   * `onDone` or an error).
   */
  const [deliberationActive, setDeliberationActive] = useState(false);
  /** Set when a deliberative run resolves successfully. The overlay
   *  stays visible until the user clicks Close (which navigates away
   *  via `onDone(pendingDone)`). */
  const [pendingDone, setPendingDone] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      const ps = await ipc.listProviders();
      const privacy = await ipc.privacySettings().catch(() => null);
      setProviders(ps);
      if (privacy) {
        setDataBoundaryMode(privacy.default_data_boundary);
      }
      const ss = await ipc.listSkills(workspace.id).catch(() => []);
      setSkills(ss);
      // Pick from the clinical-eligible subset only. Without the filter
      // we could land on Apple Intelligence (Subtask scope) which the
      // backend then rejects with an opaque error.
      const eligible = usableProviders(ps).filter((p) => isClinicalEligible(p.id));
      const pick = preferredProvider(eligible);
      if (pick) {
        setProviderId(pick);
      } else if (eligible.length > 0) {
        // Defensive: preferredProvider should never return null when
        // eligible is non-empty, but if it does, fall back so the run
        // button is never silently disabled with a non-empty list.
        setProviderId(eligible[0].id);
      } else if (ps.length > 0) {
        // No eligible provider, but SOMETHING is configured. Use the
        // first id; the backend ensure_general_scope / ensure_provider_ready
        // will reject with a single clear error if the user tries to run.
        setProviderId(ps[0].id);
      }
      // else: leave providerId as "" — the disabled-button tooltip
      // explains the empty state.
    })();
  }, [workspace.id]);

  // Merge any incoming page-level drag-drop payload with our local
  // attachments. Cleared in the parent after we've integrated it.
  useEffect(() => {
    if (!incomingAttachments || incomingAttachments.length === 0) return;
    setAttachments((prev) => dedupeAttachments(prev, incomingAttachments));
    onIncomingConsumed?.();
  }, [incomingAttachments, onIncomingConsumed]);

  // Cmd/Ctrl+Enter inside NewCase triggers `run`. We mount the listener
  // on `window` so the shortcut works regardless of which sub-field has
  // focus — clinicians often hit it while still typing in the textarea.
  // We use a ref to `run` so the latest closure (with up-to-date state)
  // fires even though we don't re-bind the listener on every keystroke.
  const runRef = useRef<() => void>(() => {});
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        runRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // When the user opens a draft from the list, fetch its persisted
  // attachments so we can render them read-only above the dropzone.
  useEffect(() => {
    if (!draft) {
      setDraftAttachments([]);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const list = await ipc.listCaseAttachments(workspace.id, draft.case.id);
        if (!cancelled) setDraftAttachments(list);
      } catch {
        if (!cancelled) setDraftAttachments([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [draft, workspace.id]);

  const pickFiles = async () => {
    const picked = await openDialog({
      multiple: true,
      directory: false,
      title: t("cases.attachment_pick_title"),
      filters: [
        {
          name: t("cases.attachment_filter_label"),
          extensions: SUPPORTED_ATTACHMENT_EXTS as unknown as string[],
        },
      ],
    });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    const accepted: PendingAttachment[] = [];
    for (const p of list) {
      const a = attachmentFromPath(String(p));
      if (a) accepted.push(a);
    }
    if (accepted.length > 0) {
      setAttachments((prev) => dedupeAttachments(prev, accepted));
    }
  };

  const removeAttachment = (path: string) => {
    setAttachments((prev) => prev.filter((a) => a.path !== path));
  };

  // Case deliberation is the highest-risk clinical surface, so the
  // picker only lists providers that are scope=`general`. Subtask-only
  // providers (Apple Intelligence today) are hidden here even when they
  // are otherwise usable.
  const usable = useMemo(
    () => usableProviders(providers).filter((p) => isClinicalEligible(p.id)),
    [providers],
  );

  useEffect(() => {
    if (dataBoundaryMode === "local_only" && useOnlineEvidence) {
      setUseOnlineEvidence(false);
    }
  }, [dataBoundaryMode, useOnlineEvidence]);

  useEffect(() => {
    if (!providerId) {
      setBoundaryPreview(null);
      return;
    }
    let cancelled = false;
    const paths = attachments.map((a) => a.path);
    void ipc
      .previewDataBoundary({
        workspace_id: workspace.id,
        text,
        question,
        provider_id: providerId,
        attached_file_paths: paths,
        data_boundary_mode: dataBoundaryMode,
        allow_phi_payload: allowPhiPayload,
        retain_raw_text: retainRawText,
        active_skill_id: activeSkillId || undefined,
        use_online_evidence: useOnlineEvidence,
      })
      .then((preview) => {
        if (!cancelled) setBoundaryPreview(preview);
      })
      .catch(() => {
        if (!cancelled) setBoundaryPreview(null);
      });
    return () => {
      cancelled = true;
    };
  }, [
    activeSkillId,
    allowPhiPayload,
    attachments,
    dataBoundaryMode,
    providerId,
    question,
    retainRawText,
    text,
    useOnlineEvidence,
    workspace.id,
  ]);

  const run = async () => {
    const hasDraftAttachments = draftAttachments.length > 0;
    if (!text.trim() && attachments.length === 0 && !hasDraftAttachments) {
      return;
    }
    if (!providerId) {
      setError(t("cases.no_provider_configured"));
      return;
    }
    if (boundaryPreview?.blocked_reason) {
      setError(boundaryPreview.blocked_reason);
      return;
    }
    setBusy(true);
    setError(null);
    if (deliberative && !draft) setDeliberationActive(true);
    try {
      if (draft) {
        // Promote the existing draft. New drops/picks the clinician made
        // in this session are NOT carried over for now — they need to be
        // attached to a fresh case. (A follow-up could persist them via
        // an `add_attachments_to_case` command.) Draft promotion runs
        // through the quick pipeline; deliberative-from-draft is a
        // follow-up.
        const resp = await ipc.runDraftCase({
          workspace_id: workspace.id,
          case_id: draft.case.id,
          provider_id: providerId,
          text,
          question,
          data_boundary_mode: dataBoundaryMode,
          allow_phi_payload: allowPhiPayload,
          retain_raw_text: retainRawText,
          active_skill_id: activeSkillId || undefined,
          use_online_evidence: useOnlineEvidence,
        });
        onDone(resp.case.id);
      } else if (deliberative) {
        const resp = await ipc.runCaseDeliberated({
          workspace_id: workspace.id,
          text,
          question,
          provider_id: providerId,
          attached_file_paths: attachments.map((a) => a.path),
          data_boundary_mode: dataBoundaryMode,
          allow_phi_payload: allowPhiPayload,
          retain_raw_text: retainRawText,
          active_skill_id: activeSkillId || undefined,
          use_online_evidence: useOnlineEvidence,
        });
        // For deliberative runs we keep the overlay alive so the user
        // can review per-phase output / explicitly Close. `onDone`
        // navigates away — call it from the overlay's dismiss button
        // instead. Stash the case id and let the user pick.
        setPendingDone(resp.case.id);
      } else {
        const resp = await ipc.runCase({
          workspace_id: workspace.id,
          text,
          question,
          provider_id: providerId,
          attached_file_paths: attachments.map((a) => a.path),
          data_boundary_mode: dataBoundaryMode,
          allow_phi_payload: allowPhiPayload,
          retain_raw_text: retainRawText,
          active_skill_id: activeSkillId || undefined,
          use_online_evidence: useOnlineEvidence,
        });
        onDone(resp.case.id);
      }
    } catch (e) {
      setError(String(e));
      // Failed deliberative runs: collapse the overlay since there's
      // no successful verdict to stage.
      if (deliberative && !draft) setDeliberationActive(false);
    } finally {
      setBusy(false);
      // For quick mode the overlay was never opened, so this is a no-op
      // there. For deliberative mode the overlay sticks until the user
      // explicitly dismisses it via the footer.
      if (!deliberative || draft) setDeliberationActive(false);
    }
  };

  // Keep runRef pointing at the latest `run` so the Cmd+Enter listener
  // dispatches with up-to-date state.
  useEffect(() => {
    runRef.current = () => {
      void run();
    };
  });

  return (
    <div className="mx-auto w-full max-w-3xl space-y-4 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onCancel}>
          {t("cases.back")}
        </Button>
      </div>
      <Card>
        <CardHeader
          title={t("cases.new_title")}
          subtitle={t("cases.new_subtitle")}
        />
        <CardBody className="space-y-4">
          {error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          {draft && (
            <div className="rounded-md border border-violet-400/40 bg-violet-400/5 px-3 py-2 text-[12.5px] text-violet-200">
              <div className="font-semibold">{t("cases.draft_banner_title")}</div>
              <div className="mt-0.5 text-ink-dim">
                {t("cases.draft_banner_body")}
              </div>
            </div>
          )}
          {draft && draftAttachments.length > 0 && (
            <Field label={t("cases.attachments_section_title")}>
              <ul className="space-y-1.5">
                {draftAttachments.map((a) => (
                  <li
                    key={a.id}
                    className="flex items-center gap-2 rounded-md border border-border-subtle bg-bg px-2.5 py-1.5"
                  >
                    <span
                      className={cn(
                        "shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] uppercase",
                        attachmentBadgeColor(a.doc_type),
                      )}
                    >
                      {a.doc_type === "image" ? "img" : a.doc_type}
                    </span>
                    <span className="shrink-0 rounded bg-violet-400/15 px-1.5 py-0.5 font-mono text-[10.5px] text-violet-200">
                      A{a.position}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[12.5px] text-ink-dim">
                      {a.original_filename}
                    </span>
                    <span className="shrink-0 text-[11px] text-ink-faint">
                      {formatBytes(a.byte_size)}
                    </span>
                  </li>
                ))}
              </ul>
            </Field>
          )}
          <div className="flex items-start gap-2.5 rounded-md border border-ok/30 bg-ok/5 px-3 py-2 text-[12.5px] leading-relaxed text-ink-dim">
            <IconLock
              aria-hidden="true"
              size={14}
              stroke={1.6}
              className="mt-0.5 shrink-0 text-ok"
            />
            <p className="min-w-0">
              <Trans
                i18nKey="cases.privacy_banner"
                components={[
                  <strong key="0" className="font-semibold text-ink" />,
                ]}
              />
            </p>
          </div>
          <Field label={t("cases.field_text")}>
            <Textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={14}
              placeholder={t("cases.field_text_placeholder")}
            />
          </Field>
          {!draft && (
            <NewCaseAttachments
              attachments={attachments}
              onBrowse={pickFiles}
              onRemove={removeAttachment}
            />
          )}
          <Field label={t("cases.field_question")}>
            <Input value={question} onChange={(e) => setQuestion(e.target.value)} />
          </Field>
          <ProviderField
            providers={usable}
            providerId={providerId}
            onChange={setProviderId}
            onGoToSettings={onGoToSettings}
          />
          <ProviderOfflineBanner providers={providers} providerId={providerId} />
          <div className="grid gap-3 md:grid-cols-2">
            <Field label={t("cases.data_boundary_label")}>
              <select
                value={dataBoundaryMode}
                onChange={(e) =>
                  setDataBoundaryMode(e.target.value as DataBoundaryMode)
                }
                className="w-full rounded-md border border-border-subtle bg-bg px-3 py-2 text-[13px] text-ink outline-none focus:ring-2 focus:ring-accent/40"
              >
                <option value="deid_cloud">
                  {t("cases.data_boundary.deid_cloud")}
                </option>
                <option value="local_only">
                  {t("cases.data_boundary.local_only")}
                </option>
                <option value="explicit_phi">
                  {t("cases.data_boundary.explicit_phi")}
                </option>
              </select>
            </Field>
            <Field label={t("cases.skill_label")}>
              <select
                value={activeSkillId}
                onChange={(e) => setActiveSkillId(e.target.value)}
                className="w-full rounded-md border border-border-subtle bg-bg px-3 py-2 text-[13px] text-ink outline-none focus:ring-2 focus:ring-accent/40"
              >
                <option value="">{t("cases.skill_none")}</option>
                {skills.map((skill) => (
                  <option key={skill.id} value={skill.id}>
                    {skill.title}
                  </option>
                ))}
              </select>
            </Field>
          </div>
          {dataBoundaryMode === "explicit_phi" && (
            <label className="flex items-center gap-2 rounded-md border border-amber-400/30 bg-amber-400/5 px-3 py-2 text-[12.5px] text-ink-dim">
              <input
                type="checkbox"
                checked={allowPhiPayload}
                onChange={(e) => setAllowPhiPayload(e.target.checked)}
              />
              <span>{t("cases.allow_phi_payload")}</span>
            </label>
          )}
          <label className="flex items-center gap-2 rounded-md border border-border-subtle bg-bg px-3 py-2 text-[12.5px] text-ink-dim">
            <input
              type="checkbox"
              checked={retainRawText}
              onChange={(e) => setRetainRawText(e.target.checked)}
            />
            <span>{t("cases.retain_raw_text")}</span>
          </label>
          <label className="flex items-center gap-2 rounded-md border border-border-subtle bg-bg px-3 py-2 text-[12.5px] text-ink-dim">
            <input
              type="checkbox"
              checked={useOnlineEvidence}
              disabled={dataBoundaryMode === "local_only"}
              onChange={(e) => setUseOnlineEvidence(e.target.checked)}
            />
            <span>{t("cases.use_online_evidence")}</span>
          </label>
          {boundaryPreview && (
            <div
              className={cn(
                "rounded-md border px-3 py-2 text-[12px] leading-relaxed",
                boundaryPreview.blocked_reason
                  ? "border-danger/40 bg-danger/10 text-danger"
                  : "border-border-subtle bg-bg-subtle text-ink-dim",
              )}
            >
              {boundaryPreview.blocked_reason ??
                t("cases.data_boundary_preview", {
                  mode: t(`cases.data_boundary.${boundaryPreview.mode}`),
                  provider: boundaryPreview.provider_id,
                  images: boundaryPreview.sends_images
                    ? t("common.yes")
                    : t("common.no"),
                  online: boundaryPreview.uses_online_evidence
                    ? t("common.yes")
                    : t("common.no"),
                  retention: t(
                    `cases.raw_retention.${boundaryPreview.stores_raw_text ? "explicit_retained" : "discarded"}`,
                  ),
                  attachments: t(
                    attachments.length + draftAttachments.length === 0
                      ? "cases.attachments_retention.none"
                      : boundaryPreview.retains_attachment_files
                        ? "cases.attachments_retention.kept"
                        : "cases.attachments_retention.purged",
                  ),
                })}
            </div>
          )}
          {!draft && (
            <ModeSelector
              checked={deliberative}
              onChange={setDeliberative}
            />
          )}
          <div className="flex justify-end pt-1">
            {/* When the selected provider isn't `ready` we both
                disable the button AND surface a tooltip telling the
                clinician why. Stops the "click run → wait → fail"
                round-trip that used to be the only feedback channel
                for an expired OAuth session. */}
            {(() => {
              const selectedProvider = providers.find(
                (p) => p.id === providerId,
              );
              const providerNotReady =
                !!selectedProvider && !isReady(selectedProvider);
              const tooltip = providerNotReady
                ? t("cases.run_disabled_provider_not_ready")
                : undefined;
              return (
                <Button
                  variant="primary"
                  onClick={run}
                  loading={busy}
                  title={tooltip}
                  disabled={
                    (!text.trim() &&
                      attachments.length === 0 &&
                      draftAttachments.length === 0) ||
                    !providerId ||
                    providerNotReady
                  }
                >
                  {draft
                    ? t("cases.draft_run_button")
                    : deliberative
                      ? t("cases.run_button_deliberative")
                      : t("cases.run_button")}
                </Button>
              );
            })()}
          </div>
        </CardBody>
      </Card>
      {deliberationActive && (
        <DeliberationOverlay
          provider={providers.find((p) => p.id === providerId) ?? null}
          onDismiss={() => {
            setDeliberationActive(false);
            // If the run resolved successfully while the user was
            // reviewing the trace, navigate to the verdict on close.
            if (pendingDone) {
              const id = pendingDone;
              setPendingDone(null);
              onDone(id);
            }
          }}
        />
      )}
    </div>
  );
}

// Provider field for the new-case form.
//
// Adapts to the single-active-provider rule:
//   • 0 usable → empty state with CTA back to Settings
//   • 1 usable → readonly summary chip + change link
//   • 2+      → labelled <select> with friendly names
function ShowCase({
  workspace,
  detail: initialDetail,
  onBack,
}: {
  workspace: Workspace;
  /** `null` during the optimistic transition while showCase is still
   *  fetching — we render a skeleton placeholder so the user sees an
   *  immediate response instead of a blank page. */
  detail: CaseDetail | null;
  onBack: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [pdfOptionsOpen, setPdfOptionsOpen] = useState(false);
  const [localDetail, setLocalDetail] = useState<CaseDetail | null>(initialDetail);
  const [purgePhiAnchor, setPurgePhiAnchor] = useState<HTMLElement | null>(null);
  const [purgePhiError, setPurgePhiError] = useState<string | null>(null);
  const [purgeAttachmentsAnchor, setPurgeAttachmentsAnchor] =
    useState<HTMLElement | null>(null);
  const [purgeAttachmentsError, setPurgeAttachmentsError] = useState<string | null>(null);

  useEffect(() => {
    setLocalDetail(initialDetail);
  }, [initialDetail]);

  const feedback = async (kind: "accept" | "modify" | "reject") => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.submitFeedback({
        workspace_id: workspace.id,
        case_id: current.case.id,
        kind,
      });
      alert(t("cases.feedback_recorded", { kind }));
      const refreshed = await ipc.showCase(workspace.id, current.case.id);
      if (refreshed) setLocalDetail(refreshed);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const purgePhi = async () => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setPurgePhiError(null);
    try {
      const purged = await ipc.purgeCasePhi(workspace.id, current.case.id);
      setLocalDetail({ ...current, case: purged });
      setPurgePhiAnchor(null);
    } catch (e) {
      setPurgePhiError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const purgeAttachments = async () => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setPurgeAttachmentsError(null);
    try {
      await ipc.purgeCaseAttachments(workspace.id, current.case.id);
      const refreshed = await ipc.showCase(workspace.id, current.case.id);
      if (refreshed) setLocalDetail(refreshed);
      setPurgeAttachmentsAnchor(null);
    } catch (e) {
      setPurgeAttachmentsError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const exportPdf = async () => {
    const current = localDetail;
    if (!current?.verdict) return;
    setExporting(true);
    setError(null);
    try {
      await exportCaseVerdictToPDF(current, t, i18n.language, loadPdfExportOptions());
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  };

  if (!localDetail) {
    return <ShowCaseSkeleton onBack={onBack} />;
  }

  const detail = localDetail;

  return (
    <div className="mx-auto w-full max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onBack}>
          {t("cases.back")}
        </Button>
        {detail.verdict && (
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setPdfOptionsOpen(true)}
              disabled={busy}
              aria-label={t("cases.export_options")}
              title={t("cases.export_options")}
            >
              <IconAdjustments size={15} stroke={1.6} />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={exportPdf}
              loading={exporting}
              disabled={busy}
            >
              <IconFileTypePdf size={14} className="mr-1" />
              {t("cases.export_pdf")}
            </Button>
            {detail.case.raw_text_retention !== "discarded" && (
              <Button
                size="sm"
                variant="ghost"
                onClick={(e) => {
                  setPurgePhiError(null);
                  setPurgePhiAnchor(e.currentTarget);
                }}
                loading={busy && purgePhiAnchor !== null}
              >
                {t("cases.purge_phi")}
              </Button>
            )}
            {detail.attachments.some((a) => a.stored_path) && (
              <Button
                size="sm"
                variant="ghost"
                onClick={(e) => {
                  setPurgeAttachmentsError(null);
                  setPurgeAttachmentsAnchor(e.currentTarget);
                }}
                loading={busy && purgeAttachmentsAnchor !== null}
              >
                {t("cases.purge_attachments")}
              </Button>
            )}
            <Button size="sm" onClick={() => feedback("accept")} loading={busy}>
              {t("cases.accept")}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => feedback("modify")} loading={busy}>
              {t("cases.modify")}
            </Button>
            <Button size="sm" variant="danger" onClick={() => feedback("reject")} loading={busy}>
              {t("cases.reject")}
            </Button>
          </div>
        )}
      </div>

      <ConfirmPurgePopover
        open={purgePhiAnchor !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPurgePhiAnchor(null);
            setPurgePhiError(null);
          }
        }}
        anchor={purgePhiAnchor}
        busy={busy && purgePhiAnchor !== null}
        error={purgePhiError}
        title={t("cases.purge_phi_confirm_title")}
        lines={[
          t("cases.purge_phi_removed"),
          t("cases.purge_phi_kept"),
          t("cases.purge_phi_irreversible"),
        ]}
        confirmLabel={t("cases.purge_phi_confirm_action")}
        onConfirm={purgePhi}
      />

      <ConfirmPurgePopover
        open={purgeAttachmentsAnchor !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPurgeAttachmentsAnchor(null);
            setPurgeAttachmentsError(null);
          }
        }}
        anchor={purgeAttachmentsAnchor}
        busy={busy && purgeAttachmentsAnchor !== null}
        error={purgeAttachmentsError}
        title={t("cases.purge_attachments_confirm_title")}
        lines={[
          t("cases.purge_attachments_removed"),
          t("cases.purge_attachments_kept"),
          t("cases.purge_attachments_original_safe"),
        ]}
        confirmLabel={t("cases.purge_attachments_confirm_action")}
        onConfirm={purgeAttachments}
      />

      <PdfOptionsSheet open={pdfOptionsOpen} onOpenChange={setPdfOptionsOpen} />

      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
          {error}
        </div>
      )}

      <Card>
        <CardHeader
          title={
            detail.case.patient_label ||
            detail.case.question ||
            t("cases.no_question")
          }
          subtitle={
            detail.case.patient_label && detail.case.question
              ? `${detail.case.question} · ${new Date(detail.case.created_at).toLocaleString()}`
              : `${detail.case.id} · ${new Date(detail.case.created_at).toLocaleString()}`
          }
          right={
            detail.verdict_record && (
              <span className="text-[12px] text-ink-faint">
                {metaFor(detail.verdict_record.provider_id).name} ·{" "}
                {detail.verdict_record.model} ·{" "}
                {detail.verdict_record.latency_ms}ms
              </span>
            )
          }
        />
        <CardBody className="space-y-6 prose-conclave">
          {detail.case.status === "failed" && detail.case.latest_error && (
            <FailedCaseErrorBlock error={detail.case.latest_error} />
          )}
          {detail.review && (
            <div className="rounded-md border border-ok/30 bg-ok/5 px-3 py-2 text-[12.5px] text-ok">
              {t("cases.review_finalized", {
                decision: detail.review.decision,
                date: new Date(detail.review.reviewed_at).toLocaleString(),
              })}
              {detail.review.diff_summary && (
                <div className="mt-1 text-ink-dim">{detail.review.diff_summary}</div>
              )}
            </div>
          )}
          {detail.verdict ? (
            <VerdictRenderer verdict={detail.verdict} />
          ) : (
            <p className="text-[13px] text-ink-subtle">
              {detail.case.status === "failed"
                ? t("cases.no_verdict_failed")
                : t("cases.no_verdict")}
            </p>
          )}
        </CardBody>
      </Card>

      {detail.verdict_record && (
        <DeliberationTraceAccordion
          workspaceId={workspace.id}
          verdictId={detail.verdict_record.id}
        />
      )}

      {detail.audit && (
        <Card>
          <CardHeader
            title={t("cases.audit_title")}
            subtitle={`${detail.audit.provider_id} · ${detail.audit.model}`}
          />
          <CardBody>
            <div className="grid gap-2 text-[12px] text-ink-dim md:grid-cols-2">
              <AuditRow label={t("cases.audit_mode")} value={detail.audit.data_boundary_mode} />
              <AuditRow label={t("cases.audit_payload")} value={detail.audit.payload_mode} />
              <AuditRow label={t("cases.audit_prompt_hash")} value={detail.audit.prompt_sha256} mono />
              <AuditRow label={t("cases.audit_output_hash")} value={detail.audit.output_sha256} mono />
              <AuditRow label={t("cases.audit_raw_retention")} value={detail.audit.raw_text_retention} />
              <AuditRow
                label={t("cases.audit_refs")}
                value={[
                  ...detail.audit.evidence_refs,
                  ...detail.audit.attachment_refs,
                  ...detail.audit.past_cases_refs,
                  ...detail.audit.online_evidence_refs,
                ].join(", ") || "—"}
              />
            </div>
          </CardBody>
        </Card>
      )}

      <Card>
        <CardHeader
          title={t("cases.attachments_section_title")}
          subtitle={t("cases.attachments_section_subtitle")}
        />
        <CardBody>
          <p className="mb-3 text-[12px] leading-relaxed text-ink-faint">
            {t("cases.attachments_storage_hint")}
          </p>
          <CaseAttachmentsSection
            workspaceId={workspace.id}
            caseId={detail.case.id}
          />
        </CardBody>
      </Card>

      <Card>
        <CardHeader title={t("cases.masked_text_title")} />
        <CardBody>
          <pre className="max-h-[300px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg p-3 font-mono text-[12px] leading-relaxed text-ink-dim">
            {detail.case.masked_text}
          </pre>
        </CardBody>
      </Card>
    </div>
  );
}

function AuditRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md border border-border-subtle bg-bg px-2.5 py-2">
      <div className="text-[10.5px] uppercase tracking-wide text-ink-faint">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 min-w-0 truncate text-ink",
          mono && "font-mono text-[11px]",
        )}
        title={value}
      >
        {value}
      </div>
    </div>
  );
}

function NewCaseAttachments({
  attachments,
  onBrowse,
  onRemove,
}: {
  attachments: PendingAttachment[];
  onBrowse: () => void;
  onRemove: (path: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={t("cases.attachments_label")}>
      <div className="rounded-lg border border-dashed border-border-subtle bg-bg-subtle px-3 py-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="text-[12.5px] text-ink-subtle">
            <Trans
              i18nKey="cases.attachments_drop_hint"
              components={[
                <span key="0" className="font-medium text-ink-dim" />,
              ]}
            />
          </div>
          <Button size="sm" variant="ghost" onClick={onBrowse}>
            {t("cases.attachments_browse")}
          </Button>
        </div>
        {attachments.length > 0 && (
          <ul className="mt-3 space-y-1.5">
            {attachments.map((a, i) => (
              <li
                key={a.path}
                className="flex items-center gap-2 rounded-md border border-border-subtle bg-bg px-2.5 py-1.5"
              >
                <span
                  className={cn(
                    "shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] uppercase",
                    attachmentBadgeColor(a.isImage ? "image" : a.ext),
                  )}
                >
                  {a.isImage ? "img" : a.ext}
                </span>
                <span className="min-w-0 flex-1 truncate text-[12.5px] text-ink-dim">
                  {a.name}
                </span>
                {a.isImage && (
                  <span className="shrink-0 rounded bg-amber-400/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-200">
                    {t("cases.attachment_image_hint")}
                  </span>
                )}
                <span className="shrink-0 font-mono text-[10.5px] text-ink-faint">
                  A{i + 1}
                </span>
                <button
                  type="button"
                  onClick={() => onRemove(a.path)}
                  className="shrink-0 rounded p-1 text-ink-faint transition hover:bg-surface hover:text-ink"
                  aria-label={t("cases.attachment_remove")}
                  title={t("cases.attachment_remove")}
                >
                  <IconX size={14} stroke={1.7} aria-hidden />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Field>
  );
}

function CaseAttachmentsSection({
  workspaceId,
  caseId,
}: {
  workspaceId: string;
  caseId: string;
}) {
  const { t } = useTranslation();
  const [items, setItems] = useState<CaseAttachment[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await ipc.listCaseAttachments(workspaceId, caseId);
        if (!cancelled) setItems(list);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [workspaceId, caseId]);

  if (error) {
    return (
      <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[12px] text-danger">
        {error}
      </div>
    );
  }
  if (items === null) {
    return (
      <p className="text-[12px] text-ink-faint">
        {t("cases.attachments_loading")}
      </p>
    );
  }
  if (items.length === 0) {
    return (
      <p className="text-[12px] text-ink-faint">
        {t("cases.attachments_empty")}
      </p>
    );
  }
  const toggle = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };
  return (
    <ul className="space-y-2">
      {items.map((a) => {
        const isOpen = expanded.has(a.id);
        return (
          <li
            key={a.id}
            className="rounded-md border border-border-subtle bg-bg px-3 py-2"
          >
            <div className="flex items-center gap-2">
              <span
                className={cn(
                  "shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] uppercase",
                  attachmentBadgeColor(a.doc_type),
                )}
              >
                {a.doc_type === "image" ? "img" : a.doc_type}
              </span>
              <span className="shrink-0 rounded bg-violet-400/15 px-1.5 py-0.5 font-mono text-[11px] text-violet-200">
                A{a.position}
              </span>
              <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
                {a.original_filename}
              </span>
              {a.stored_path ? (
                <span className="shrink-0 text-[11px] text-ink-faint">
                  {formatBytes(a.byte_size)}
                </span>
              ) : (
                <span
                  className="shrink-0 rounded bg-surface px-1.5 py-0.5 text-[10px] font-medium text-ink-subtle"
                  title={t("cases.purge_attachments_kept")}
                >
                  {t("cases.attachment_purged_badge")}
                </span>
              )}
              {a.needs_ocr && (
                <span
                  className="shrink-0 rounded bg-warn/15 px-1.5 py-0.5 text-[10px] font-medium text-warn"
                  title={t("cases.attachment_needs_ocr_hint")}
                >
                  {t("cases.attachment_needs_ocr_badge")}
                </span>
              )}
              {a.extracted_text && (
                <button
                  type="button"
                  onClick={() => toggle(a.id)}
                  className="shrink-0 rounded-md px-2 py-0.5 text-[11px] text-ink-subtle transition hover:bg-surface hover:text-ink"
                >
                  {isOpen
                    ? t("cases.attachment_hide_text")
                    : t("cases.attachment_show_text")}
                </button>
              )}
            </div>
            {isOpen && a.extracted_text && (
              <pre className="mt-2 max-h-[260px] overflow-auto whitespace-pre-wrap rounded border border-border-subtle bg-surface p-2 font-mono text-[11.5px] leading-relaxed text-ink-dim">
                {a.extracted_text}
              </pre>
            )}
            {a.needs_ocr && !a.extracted_text && (
              <p className="mt-1 text-[11.5px] text-ink-faint">
                {t("cases.attachment_no_text_explanation")}
              </p>
            )}
          </li>
        );
      })}
    </ul>
  );
}
