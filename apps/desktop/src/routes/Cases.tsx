import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  IconCheck,
  IconCopy,
  IconLock,
  IconRefresh,
  IconTrash,
  IconX,
} from "@tabler/icons-react";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input, Textarea } from "../components/Field";
import { Sheet } from "../components/Sheet";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type BatchCaseInput,
  type BatchEvent,
  type CaseAttachment,
  type CaseDetail,
  type CaseDraftedEvent,
  type CaseRecord,
  type DeliberationEvent,
  type DeliberationPhase,
  type DeliberationTrace,
  type ProviderInfo,
  type Verdict,
  type Workspace,
} from "../lib/ipc";
import { isClinicalEligible, metaFor, preferredProvider } from "../lib/providers";

// Extensions we accept when the user drops or picks files for a case.
// Mirrors `apps/desktop/src-tauri/src/batch.rs::ATTACHMENT_EXTS` so the
// frontend filter and the backend extractor agree on what counts.
const SUPPORTED_ATTACHMENT_EXTS = [
  "pdf",
  "docx",
  "txt",
  "md",
  "markdown",
  "html",
  "htm",
  "png",
  "jpg",
  "jpeg",
  "webp",
  "tif",
  "tiff",
  "heic",
  "heif",
] as const;

type PendingAttachment = {
  path: string;
  name: string;
  ext: string;
  isImage: boolean;
};

function attachmentFromPath(path: string): PendingAttachment | null {
  const segments = path.split(/[\\/]/);
  const name = segments[segments.length - 1] || path;
  const dot = name.lastIndexOf(".");
  if (dot === -1) return null;
  const ext = name.slice(dot + 1).toLowerCase();
  if (!SUPPORTED_ATTACHMENT_EXTS.includes(ext as (typeof SUPPORTED_ATTACHMENT_EXTS)[number]))
    return null;
  const isImage = ["png", "jpg", "jpeg", "webp", "tif", "tiff", "heic", "heif"].includes(
    ext,
  );
  return { path, name, ext, isImage };
}

function dedupeAttachments(
  base: PendingAttachment[],
  incoming: PendingAttachment[],
): PendingAttachment[] {
  const seen = new Set(base.map((a) => a.path));
  const out = [...base];
  for (const a of incoming) {
    if (!seen.has(a.path)) {
      out.push(a);
      seen.add(a.path);
    }
  }
  return out;
}

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function attachmentBadgeColor(extOrType: string): string {
  switch (extOrType) {
    case "pdf":
      return "bg-rose-400/15 text-rose-200";
    case "docx":
      return "bg-sky-400/15 text-sky-200";
    case "image":
    case "png":
    case "jpg":
    case "jpeg":
    case "webp":
    case "tif":
    case "tiff":
    case "heic":
    case "heif":
      return "bg-amber-400/15 text-amber-200";
    case "txt":
    case "md":
    case "markdown":
      return "bg-emerald-400/15 text-emerald-200";
    case "html":
    case "htm":
      return "bg-indigo-400/15 text-indigo-200";
    default:
      return "bg-slate-400/15 text-slate-200";
  }
}

/**
 * Status badge shown next to every row in the cases list. Five visual
 * states:
 *   - `running` (transient frontend-only overlay): accent + animated dot
 *   - `draft`: violet
 *   - `completed`: green
 *   - `failed`: red
 *
 * The `running` flag is supplied by the page-level batch:progress
 * listener; the DB-side status remains `draft` until the LLM call
 * completes. We render `running` last so it wins over any underlying
 * draft state during the brief overlap.
 */
/** Detect labels that are still the filename-stem fallback (e.g.
 *  "CR-IA-007", "case_recto_bajo_alto_riesgo") rather than a proper
 *  patient summary from Apple Intelligence ("Mujer 67, recto bajo T3N1"
 *  — sentence-shaped, has spaces and commas).
 *
 *  Heuristic: empty, or zero spaces, or contains underscores. AI
 *  summaries always have at least one space and never use underscores
 *  in our prompt template. */
function isFallbackLabel(label: string | null | undefined): boolean {
  if (!label) return true;
  const trimmed = label.trim();
  if (trimmed.length === 0) return true;
  // Filename stems and "CR-IA-XXX" codes have no spaces.
  if (!/\s/.test(trimmed)) return true;
  // Any underscore means it survived from a file name.
  if (trimmed.includes("_")) return true;
  return false;
}

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

function StatusBadge({
  status,
  running,
}: {
  status: CaseRecord["status"];
  running: boolean;
}) {
  const { t } = useTranslation();
  if (running) {
    return (
      <span className="flex items-center gap-1 rounded bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent">
        <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
        {t("cases.status.running")}
      </span>
    );
  }
  if (status === "completed") {
    return (
      <span className="rounded bg-ok/15 px-2 py-0.5 text-[11px] font-medium text-ok">
        {t("cases.status.completed")}
      </span>
    );
  }
  if (status === "draft") {
    return (
      <span className="rounded bg-violet-400/15 px-2 py-0.5 text-[11px] font-medium text-violet-200">
        {t("cases.status.draft")}
      </span>
    );
  }
  return (
    <span className="rounded bg-danger/15 px-2 py-0.5 text-[11px] font-medium text-danger">
      {t("cases.status.failed")}
    </span>
  );
}

type View = "list" | "new" | "show";

type SortBy = "date_desc" | "date_asc" | "question_az" | "status";
type GroupBy = "off" | "day" | "week" | "month";

// Anchor a date to the start of its bucket (used both as map key and as
// the value we feed into bucketLabel — so the displayed name aligns with
// the rows it groups).
function bucketAnchor(iso: string, mode: GroupBy): Date {
  const d = new Date(iso);
  d.setHours(0, 0, 0, 0);
  if (mode === "day" || mode === "off") return d;
  if (mode === "week") {
    // Monday-anchored week. JS getDay() returns 0 for Sunday → treat as 7.
    const dow = d.getDay() || 7;
    d.setDate(d.getDate() - (dow - 1));
    return d;
  }
  // month
  d.setDate(1);
  return d;
}

function bucketKey(iso: string, mode: GroupBy): string {
  if (mode === "off") return "all";
  const d = bucketAnchor(iso, mode);
  if (mode === "month") return `${d.getFullYear()}-${d.getMonth() + 1}`;
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

function bucketLabel(
  iso: string,
  mode: GroupBy,
  t: TFunction,
  locale: string,
): string {
  if (mode === "off") return "";
  const anchor = bucketAnchor(iso, mode);
  const todayAnchor = bucketAnchor(new Date().toISOString(), mode);

  if (mode === "day") {
    if (anchor.getTime() === todayAnchor.getTime()) return t("cases.group_bucket.today");
    const yesterday = new Date(todayAnchor);
    yesterday.setDate(yesterday.getDate() - 1);
    if (anchor.getTime() === yesterday.getTime()) return t("cases.group_bucket.yesterday");
    return new Intl.DateTimeFormat(locale, {
      weekday: "long",
      day: "numeric",
      month: "long",
      year:
        anchor.getFullYear() !== todayAnchor.getFullYear() ? "numeric" : undefined,
    }).format(anchor);
  }

  if (mode === "week") {
    if (anchor.getTime() === todayAnchor.getTime()) return t("cases.group_bucket.this_week");
    const lastWeek = new Date(todayAnchor);
    lastWeek.setDate(lastWeek.getDate() - 7);
    if (anchor.getTime() === lastWeek.getTime()) return t("cases.group_bucket.last_week");
    const endOfWeek = new Date(anchor);
    endOfWeek.setDate(endOfWeek.getDate() + 6);
    const fmt = new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" });
    const fmtYear = new Intl.DateTimeFormat(locale, {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
    return `${fmt.format(anchor)} – ${fmtYear.format(endOfWeek)}`;
  }

  // month
  if (anchor.getTime() === todayAnchor.getTime()) return t("cases.group_bucket.this_month");
  const lastMonth = new Date(todayAnchor);
  lastMonth.setMonth(lastMonth.getMonth() - 1);
  if (anchor.getTime() === lastMonth.getTime()) return t("cases.group_bucket.last_month");
  return new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" }).format(
    anchor,
  );
}

// `<input type="datetime-local">` always works in local time and uses
// `YYYY-MM-DDTHH:mm`. We persist RFC3339 (UTC) on the wire, so convert
// in both directions.
function isoToLocalInput(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}`;
}

function localInputToIso(local: string): string {
  // The Date constructor interprets `YYYY-MM-DDTHH:mm` as local time.
  return new Date(local).toISOString();
}

/** Format an elapsed millisecond duration as `0:42` / `12:07` for
 *  short runs, and `1h 03m` once we cross the hour boundary. Tuned for
 *  the batch banner / per-row chip — they want compactness over
 *  precision. */
function formatElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0:00";
  const totalSec = Math.floor(ms / 1000);
  if (totalSec < 3600) {
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
  }
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  return `${h}h ${String(m).padStart(2, "0")}m`;
}

/** Per-case live status driven by `deliberation:progress` events. The
 *  cases list rolls this up into a chip on the row + (optionally) a
 *  detailed entry in the batch banner. Quick-mode runs never set this
 *  — they just toggle the `running` overlay. */
type LiveCasePhase = {
  phase: DeliberationPhase;
  /** ms-since-page-load when this phase started; used to render the
   *  ticking elapsed chip without a per-second re-render of the whole
   *  list (the LiveTicker child reads it via state). */
  startedAtMs: number;
  status: "active" | "done" | "failed";
};

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

  /**
   * Per-row "running" state, populated by the batch:progress listener.
   * The DB-side status during a run is still `Draft`; this overlay lets
   * the badge animate as soon as the LLM call starts and clear as soon
   * as it completes (the DB status updates to `Completed`/`Failed`
   * almost in parallel via the case_completed/case_failed event +
   * refresh).
   */
  const [runningCaseIds, setRunningCaseIds] = useState<Set<string>>(new Set());
  /**
   * Lightweight batch-mode banner: shows "X / N ejecutándose" while a
   * batch is in flight. Cleared on `batch_done`.
   */
  const [batchTotal, setBatchTotal] = useState<number | null>(null);
  const [batchDone, setBatchDone] = useState(0);
  /** Wall-clock ms when the current batch began — used to render an
   *  elapsed chip in the banner. Set by the first CaseQueued/CaseStarted
   *  event, cleared on BatchDone. */
  const [batchStartedAtMs, setBatchStartedAtMs] = useState<number | null>(null);
  /** ms it took the first batch case to finish. We use this single
   *  sample as a (rough but useful) per-case duration estimate, then
   *  project an ETA = avg × remaining cases. */
  const [batchFirstCaseMs, setBatchFirstCaseMs] = useState<number | null>(null);
  /** Per-case phase tracker driven by `deliberation:progress`. The
   *  backend stamps every event with the case id so we can render the
   *  current phase next to each running row in real time. */
  const [casePhases, setCasePhases] = useState<Map<string, LiveCasePhase>>(
    () => new Map(),
  );
  /** Per-case "retry / cancel" busy flags so we don't double-fire the
   *  same IPC. Keyed by case id; cleared once the IPC resolves. */
  const [rowBusy, setRowBusy] = useState<Map<string, "retry" | "cancel">>(
    () => new Map(),
  );
  /** Lightweight tick counter so the per-row elapsed chip refreshes
   *  every second WITHOUT re-running the full list memo. Components
   *  that don't need it ignore the value. */
  const [tickMs, setTickMs] = useState(() => Date.now());

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
  // When non-null, the delete-confirmation dialog is open for this
  // set of ids. Used both by the per-row hover trash button (length 1)
  // and by the batch toolbar (length N).
  const [deletingIds, setDeletingIds] = useState<string[] | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

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
            (c.status === "completed" ||
              c.status === "failed" ||
              c.status === "draft") &&
            next.has(c.id)
          ) {
            // If the case is back to Draft, the retry is being
            // dispatched — keep the "Retrying…" chip until a deliberation
            // event flips the row to running, OR a follow-up refresh
            // moves it to completed/failed.
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
    setPendingDrop([]);
    setUnsupportedDropError(null);
    setClassifyDialog(null);
    setRunningCaseIds(new Set());
    setBatchTotal(null);
    setBatchDone(0);
    setBatchStartedAtMs(null);
    setBatchFirstCaseMs(null);
    setCasePhases(new Map());
    setRowBusy(new Map());
  }, [workspace.id]);

  // Tick every second while a batch is running so the elapsed chips
  // update. Stops when nothing is in flight to keep React idle.
  useEffect(() => {
    if (batchStartedAtMs === null && casePhases.size === 0) return;
    const id = window.setInterval(() => setTickMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [batchStartedAtMs, casePhases.size]);

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

  // ----------------------------------------------------------------
  // Live-progress listeners for batch + draft creation.
  //
  // The backend emits `case:drafted` the moment a case row is persisted
  // (status=Draft, before the LLM call) and `batch:progress` events as
  // each case in a batch starts / completes / fails. We hook both at
  // the page level so the cases list updates in real time — the user
  // sees rows appear, animate as "ejecutando…", then settle to
  // `completado` or `fallido`.
  // ----------------------------------------------------------------
  useEffect(() => {
    let cancelled = false;
    let unlistenDrafted: UnlistenFn | undefined;
    let unlistenBatch: UnlistenFn | undefined;
    let unlistenDelib: UnlistenFn | undefined;
    (async () => {
      unlistenDrafted = await listen<CaseDraftedEvent>(
        "case:drafted",
        (msg) => {
          if (cancelled) return;
          if (msg.payload.workspace_id !== workspace.id) return;
          // A new draft appeared — refresh the list to pop it in.
          void refresh();
        },
      );
      unlistenBatch = await listen<BatchEvent>("batch:progress", (msg) => {
        if (cancelled) return;
        const ev = msg.payload;
        if (ev.kind === "case_queued") {
          setBatchTotal((prev) => (prev === null ? 1 : prev + 1));
          // Stamp the batch start the first time we see a queued event.
          setBatchStartedAtMs((prev) => prev ?? Date.now());
        } else if (ev.kind === "case_started") {
          // We don't know the case_id from this event (the batch event
          // carries patient_label, not the id). Refresh so the row is
          // there, then optimistically mark all "draft" cases with no
          // verdict as "running" via batchTotal — the LLM call may be
          // for any of them. Simpler: refresh on completion/failure
          // and skip the per-row running overlay for batch.
          void refresh();
        } else if (ev.kind === "case_completed") {
          setRunningCaseIds((prev) => {
            const next = new Set(prev);
            next.delete(ev.case_id);
            return next;
          });
          setCasePhases((prev) => {
            // Wipe phase state for this case — it's settled.
            if (!prev.has(ev.case_id)) return prev;
            const next = new Map(prev);
            next.delete(ev.case_id);
            return next;
          });
          setRowBusy((prev) => {
            if (!prev.has(ev.case_id)) return prev;
            const next = new Map(prev);
            next.delete(ev.case_id);
            return next;
          });
          setBatchDone((d) => {
            const next = d + 1;
            // Capture the FIRST completion's duration as the per-case
            // estimate. Cheap heuristic: works whether the run is
            // quick mode (≈ one LLM call) or deliberative (≈ 4 calls).
            setBatchFirstCaseMs((prev) => {
              if (prev !== null) return prev;
              const startedAt = batchStartedAtMs;
              if (startedAt === null) return prev;
              return Date.now() - startedAt;
            });
            return next;
          });
          void refresh();
        } else if (ev.kind === "case_failed") {
          setBatchDone((d) => d + 1);
          // Drop phase tracking so the failed row doesn't display a
          // stale "active" chip on top of the new error banner.
          setCasePhases((prev) => {
            if (prev.size === 0) return prev;
            return new Map();
          });
          void refresh();
        } else if (ev.kind === "case_cancelled") {
          setBatchDone((d) => d + 1);
          setCasePhases((prev) => {
            if (prev.size === 0) return prev;
            return new Map();
          });
        } else if (ev.kind === "batch_done") {
          setBatchTotal(null);
          setBatchDone(0);
          setBatchStartedAtMs(null);
          setBatchFirstCaseMs(null);
          setCasePhases(new Map());
          void refresh();
        }
      });
      // Page-level deliberation listener that tracks the CURRENT phase
      // per case id so we can render a chip on each row. The
      // DeliberationOverlay still listens to the same event for the
      // single-case flow; both consumers stay in sync because the
      // backend stamps every event with the case id.
      unlistenDelib = await listen<DeliberationEvent>(
        "deliberation:progress",
        (msg) => {
          if (cancelled) return;
          const ev = msg.payload;
          if (ev.kind === "done") {
            setCasePhases((prev) => {
              if (!prev.has(ev.case_id)) return prev;
              const next = new Map(prev);
              next.delete(ev.case_id);
              return next;
            });
            return;
          }
          setCasePhases((prev) => {
            const next = new Map(prev);
            if (ev.kind === "phase_started") {
              next.set(ev.case_id, {
                phase: ev.phase,
                startedAtMs: Date.now(),
                status: "active",
              });
            } else if (ev.kind === "phase_completed") {
              next.set(ev.case_id, {
                phase: ev.phase,
                startedAtMs:
                  prev.get(ev.case_id)?.startedAtMs ?? Date.now(),
                status: "done",
              });
            } else if (ev.kind === "phase_failed") {
              next.set(ev.case_id, {
                phase: ev.phase,
                startedAtMs:
                  prev.get(ev.case_id)?.startedAtMs ?? Date.now(),
                status: "failed",
              });
            }
            return next;
          });
        },
      );
    })();
    return () => {
      cancelled = true;
      unlistenDrafted?.();
      unlistenBatch?.();
      unlistenDelib?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspace.id]);

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
  }, []);

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
            ✕
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
          onCancelAll={() => void ipc.batchCancel()}
        />
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
              <div className="ml-auto">
                {!selectionMode ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setSelectionMode(true)}
                  >
                    {t("cases.select")}
                  </Button>
                ) : (
                  <Button size="sm" variant="ghost" onClick={exitSelection}>
                    {t("cases.cancel_selection")}
                  </Button>
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
                            setDeletingIds([c.id]);
                          }}
                          aria-label={t("cases.delete_row")}
                          title={t("cases.delete_row")}
                          // Collapsed by default (w-0 + -ml-3 cancels the
                          // parent gap-3 so the row content sits flush
                          // right). On row hover (or keyboard focus) the
                          // button grows to its natural size and the
                          // status badge slides left to make room — the
                          // shift is the affordance.
                          className={cn(
                            "grid h-7 w-0 shrink-0 place-content-center overflow-hidden rounded-md",
                            "-ml-3 text-ink-faint opacity-0",
                            "transition-[width,margin,opacity,background-color,color] duration-200 ease-out",
                            "hover:bg-danger/10 hover:text-danger",
                            "group-hover:ml-0 group-hover:w-7 group-hover:opacity-100",
                            "focus:ml-0 focus:w-7 focus:opacity-100 focus:outline-none focus-visible:ring-conclave",
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
                variant="danger"
                onClick={() => {
                  setDeleteError(null);
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

      <ConfirmDeleteSheet
        open={deletingIds !== null}
        onOpenChange={(next) => {
          if (!next) {
            setDeletingIds(null);
            setDeleteError(null);
          }
        }}
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

function EditDateSheet({
  open,
  onOpenChange,
  count,
  initialIso,
  busy,
  error,
  onApply,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  count: number;
  initialIso: string;
  busy: boolean;
  error: string | null;
  onApply: (localValue: string) => void;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState<string>(isoToLocalInput(initialIso));

  // Re-seed the input whenever the sheet (re)opens with a different
  // initial value — without this, opening, closing without saving, and
  // re-opening on a different selection would keep the old value.
  useEffect(() => {
    if (open) setValue(isoToLocalInput(initialIso));
  }, [open, initialIso]);

  const title =
    count > 1
      ? t("cases.edit_date_title_plural", { count })
      : t("cases.edit_date_title");

  return (
    <Sheet open={open} onOpenChange={onOpenChange} title={title}>
      <div className="space-y-4 px-5 py-4">
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
            {error}
          </div>
        )}
        <Field label={t("cases.edit_date_field")}>
          <input
            type="datetime-local"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink focus:outline-none focus:ring-conclave focus:border-accent"
          />
        </Field>
        <div className="flex justify-end gap-2 pt-2">
          <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            loading={busy}
            disabled={!value}
            onClick={() => onApply(value)}
          >
            {t("cases.edit_date_apply")}
          </Button>
        </div>
      </div>
    </Sheet>
  );
}

function ConfirmDeleteSheet({
  open,
  onOpenChange,
  count,
  busy,
  error,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  count: number;
  busy: boolean;
  error: string | null;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const title =
    count > 1
      ? t("cases.delete_confirm_title_plural", { count })
      : t("cases.delete_confirm_title");
  const body =
    count > 1
      ? t("cases.delete_confirm_body_plural", { count })
      : t("cases.delete_confirm_body");

  return (
    <Sheet open={open} onOpenChange={onOpenChange} title={title}>
      <div className="space-y-4 px-5 py-4">
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
            {t("cases.delete_error", { error })}
          </div>
        )}
        <p className="text-[13px] leading-relaxed text-ink-dim">{body}</p>
        <div className="flex justify-end gap-2 pt-2">
          <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="danger"
            loading={busy}
            onClick={onConfirm}
          >
            {t("cases.delete_confirm_apply")}
          </Button>
        </div>
      </div>
    </Sheet>
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
  const [providerId, setProviderId] = useState<string>("");
  const [text, setText] = useState(draft?.case.original_text ?? "");
  const [question, setQuestion] = useState(
    draft?.case.question ?? t("cases.default_question"),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [maskedPreview, setMaskedPreview] = useState<string | null>(null);
  const [preview, setPreview] = useState<{
    spanCount: number;
    strictClean: boolean;
  } | null>(null);
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
   * the committee thinks. Defaults to OFF.
   */
  const [deliberative, setDeliberative] = useState(false);
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
      setProviders(ps);
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
  }, []);

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

  const previewDeident = async () => {
    if (!text.trim()) return;
    try {
      const r = await ipc.deidentText(text);
      setMaskedPreview(r.masked_text);
      setPreview({ spanCount: r.span_count, strictClean: r.strict_clean });
    } catch (e) {
      setError(String(e));
    }
  };

  const run = async () => {
    const hasDraftAttachments = draftAttachments.length > 0;
    if (!text.trim() && attachments.length === 0 && !hasDraftAttachments) {
      return;
    }
    if (!providerId) {
      setError(t("cases.no_provider_configured"));
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
        });
        onDone(resp.case.id);
      } else if (deliberative) {
        const resp = await ipc.runCaseDeliberated({
          workspace_id: workspace.id,
          text,
          question,
          provider_id: providerId,
          attached_file_paths: attachments.map((a) => a.path),
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
    <div className="mx-auto w-full max-w-6xl space-y-4 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onCancel}>
          {t("cases.back")}
        </Button>
      </div>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-[1fr,420px]">
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
          {!draft && (
            <DeliberativeToggle
              checked={deliberative}
              onChange={setDeliberative}
            />
          )}
          <div className="flex gap-2 pt-1">
            <Button onClick={previewDeident} disabled={!text.trim()}>
              {t("cases.preview_button")}
            </Button>
            <Button
              variant="primary"
              onClick={run}
              loading={busy}
              disabled={
                (!text.trim() &&
                  attachments.length === 0 &&
                  draftAttachments.length === 0) ||
                !providerId
              }
            >
              {draft
                ? t("cases.draft_run_button")
                : deliberative
                  ? t("cases.run_button_deliberative")
                  : t("cases.run_button")}
            </Button>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title={t("cases.deid_title")}
          subtitle={t("cases.deid_subtitle")}
        />
        <CardBody>
          {preview ? (
            <div className="space-y-3">
              <div className="flex items-center gap-3 text-[12px]">
                <span className="rounded bg-surface px-2 py-0.5 text-ink-subtle">
                  {t("cases.deid_spans", { count: preview.spanCount })}
                </span>
                <span
                  className={
                    preview.strictClean
                      ? "rounded bg-ok/15 px-2 py-0.5 text-ok"
                      : "rounded bg-warn/15 px-2 py-0.5 text-warn"
                  }
                >
                  {preview.strictClean
                    ? t("cases.deid_strict_clean")
                    : t("cases.deid_strict_dirty")}
                </span>
              </div>
              <pre className="max-h-[460px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg p-3 font-mono text-[12px] leading-relaxed text-ink-dim">
                {maskedPreview}
              </pre>
            </div>
          ) : (
            <p className="text-[13px] text-ink-subtle">
              <Trans
                i18nKey="cases.deid_hint"
                components={[
                  <span key="0" className="font-medium text-ink-dim" />,
                ]}
              />
            </p>
          )}
        </CardBody>
      </Card>
      </div>
      {deliberationActive && (
        <DeliberationOverlay
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
function ProviderField({
  providers,
  providerId,
  onChange,
  onGoToSettings,
}: {
  providers: ProviderInfo[];
  providerId: string;
  onChange: (id: string) => void;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();

  if (providers.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border bg-bg-subtle p-4 text-center">
        <div className="text-[13.5px] font-medium text-ink">
          {t("cases.provider_empty_title")}
        </div>
        <p className="mx-auto mt-1 max-w-sm text-[12px] text-ink-subtle">
          {t("cases.provider_empty_body")}
        </p>
        {onGoToSettings && (
          <div className="mt-3">
            <Button size="sm" variant="primary" onClick={onGoToSettings}>
              {t("cases.provider_empty_cta")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (providers.length === 1) {
    const p = providers[0];
    const meta = metaFor(p.id);
    return (
      <Field label={t("cases.field_provider")}>
        <div
          className={cn(
            "flex items-center gap-3 rounded-lg border border-border bg-bg px-3 py-2.5",
          )}
        >
          <span
            aria-hidden
            className="grid h-8 w-8 shrink-0 place-content-center rounded-md bg-slate-400/10 text-[12px] font-semibold text-ink-dim ring-1 ring-border-subtle"
          >
            {meta.monogram}
          </span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13px] font-medium text-ink">
              {meta.name}
            </div>
            <div className="truncate text-[11.5px] text-ink-faint">
              <span className="font-mono">{p.default_model}</span>
              {" · "}
              {meta.authLabel}
            </div>
          </div>
          {onGoToSettings && (
            <button
              type="button"
              onClick={onGoToSettings}
              className="rounded-md px-2 py-1 text-[12px] text-ink-subtle transition hover:bg-surface hover:text-ink focus:outline-none focus-visible:ring-conclave"
            >
              {t("cases.provider_change_link")}
            </button>
          )}
        </div>
      </Field>
    );
  }

  return (
    <Field
      label={t("cases.field_provider")}
      hint={onGoToSettings ? undefined : t("cases.field_provider_hint")}
    >
      <select
        value={providerId}
        onChange={(e) => onChange(e.target.value)}
        className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink focus:outline-none focus:ring-conclave focus:border-accent"
      >
        {providers.map((p) => {
          const meta = metaFor(p.id);
          return (
            <option key={p.id} value={p.id}>
              {meta.name} · {p.default_model}
            </option>
          );
        })}
      </select>
      {onGoToSettings && (
        <button
          type="button"
          onClick={onGoToSettings}
          className="mt-1.5 text-[12px] text-ink-faint transition hover:text-ink focus:outline-none focus-visible:underline"
        >
          {t("cases.provider_change_link")}
        </button>
      )}
    </Field>
  );
}

function ShowCase({
  workspace,
  detail,
  onBack,
}: {
  workspace: Workspace;
  /** `null` during the optimistic transition while showCase is still
   *  fetching — we render a skeleton placeholder so the user sees an
   *  immediate response instead of a blank page. */
  detail: CaseDetail | null;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const feedback = async (kind: "accept" | "modify" | "reject") => {
    if (!detail) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.submitFeedback({
        workspace_id: workspace.id,
        case_id: detail.case.id,
        kind,
      });
      alert(t("cases.feedback_recorded", { kind }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!detail) {
    return <ShowCaseSkeleton onBack={onBack} />;
  }

  return (
    <div className="mx-auto w-full max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onBack}>
          {t("cases.back")}
        </Button>
        {detail.verdict && (
          <div className="flex gap-2">
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
                {detail.verdict_record.provider_id} · {detail.verdict_record.model} ·
                {" "}
                {detail.verdict_record.latency_ms}ms
              </span>
            )
          }
        />
        <CardBody className="space-y-6 prose-conclave">
          {detail.case.status === "failed" && detail.case.latest_error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              <div className="font-medium mb-1">
                {t("cases.failed_title")}
              </div>
              <div className="whitespace-pre-wrap break-words font-mono text-[12px]">
                {detail.case.latest_error}
              </div>
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

      <Card>
        <CardHeader
          title={t("cases.attachments_section_title")}
          subtitle={t("cases.attachments_section_subtitle")}
        />
        <CardBody>
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
                  ✕
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
              <span className="shrink-0 text-[11px] text-ink-faint">
                {formatBytes(a.byte_size)}
              </span>
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

function VerdictRenderer({ verdict }: { verdict: Verdict }) {
  const { t } = useTranslation();
  const certaintyColor =
    verdict.certainty_level === "high"
      ? "text-ok"
      : verdict.certainty_level === "medium"
        ? "text-accent"
        : "text-warn";

  const primaryRecText = `${verdict.primary_recommendation.action}\n\n${verdict.primary_recommendation.rationale}`;

  return (
    <div className="space-y-6">
      <section>
        <SectionRow
          title={t("cases.verdict.case_summary")}
          copyText={verdict.case_summary}
        />
        <p>{verdict.case_summary}</p>
      </section>

      {verdict.key_clinical_data.length > 0 && (
        <section>
          <SectionRow
            title={t("cases.verdict.key_clinical_data")}
            copyText={verdict.key_clinical_data
              .map((kv) => `${kv.label}: ${kv.value}`)
              .join("\n")}
          />
          <ul className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {verdict.key_clinical_data.map((kv, i) => (
              <li
                key={i}
                className="rounded-md border border-border-subtle bg-bg px-3 py-2"
              >
                <div className="text-[11px] uppercase tracking-wide text-ink-faint">
                  {kv.label}
                </div>
                <div className="text-[13px] text-ink-dim">{kv.value}</div>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <SectionRow
          title={t("cases.verdict.primary_recommendation")}
          copyText={primaryRecText}
        />
        <div className="border border-border-strong bg-surface px-4 py-3">
          <div className="text-[14px] font-semibold text-ink">
            {verdict.primary_recommendation.action}
          </div>
          <div className="mt-1 text-[13px] text-ink-dim">
            {verdict.primary_recommendation.rationale}
          </div>
        </div>
      </section>

      {verdict.alternatives.length > 0 && (
        <section>
          <SectionRow
            title={t("cases.verdict.alternatives")}
            copyText={verdict.alternatives
              .map((a) => `• ${a.action} — ${a.when_to_consider}`)
              .join("\n")}
          />
          <ul className="space-y-2">
            {verdict.alternatives.map((alt, i) => (
              <li
                key={i}
                className="rounded-md border border-border-subtle bg-bg px-3 py-2"
              >
                <div className="text-[13px] text-ink-dim">{alt.action}</div>
                <div className="mt-0.5 text-[12px] text-ink-faint">
                  {t("cases.verdict.alternative_when", {
                    when: alt.when_to_consider,
                  })}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <SectionRow
          title={t("cases.verdict.certainty")}
          copyText={`${verdict.certainty_level.toUpperCase()} — ${verdict.certainty_justification}`}
        />
        <div className={`text-[14px] font-semibold ${certaintyColor}`}>
          {verdict.certainty_level.toUpperCase()}
        </div>
        <p className="mt-1">{verdict.certainty_justification}</p>
      </section>

      {verdict.red_flags.length > 0 && (
        <section>
          <SectionRow
            title={t("cases.verdict.red_flags")}
            copyText={verdict.red_flags.map((rf) => `• ${rf}`).join("\n")}
          />
          <ul className="space-y-1.5">
            {verdict.red_flags.map((rf, i) => (
              <li
                key={i}
                className="rounded-md border border-warn/40 bg-warn/5 px-3 py-2 text-[13px] text-ink-dim"
              >
                ⚠ {rf}
              </li>
            ))}
          </ul>
        </section>
      )}

      {verdict.follow_up_triggers.length > 0 && (
        <section>
          <SectionRow
            title={t("cases.verdict.follow_up_triggers")}
            copyText={verdict.follow_up_triggers
              .map((tr) => `• ${tr}`)
              .join("\n")}
          />
          <ul className="list-inside list-disc space-y-1 text-[13px] text-ink-dim">
            {verdict.follow_up_triggers.map((tr, i) => (
              <li key={i}>{tr}</li>
            ))}
          </ul>
        </section>
      )}

      {verdict.applied_evidence.length > 0 && (
        <section>
          <SectionRow
            title={t("cases.verdict.applied_evidence")}
            copyText={verdict.applied_evidence
              .map((ev) => `[${ev.ref}] ${ev.claim}`)
              .join("\n")}
          />
          <ul className="space-y-1.5">
            {verdict.applied_evidence.map((ev, i) => (
              <li
                key={i}
                className="rounded-md border border-border-subtle bg-bg px-3 py-2 text-[13px] text-ink-dim"
              >
                <span className="mr-2 rounded bg-surface px-1.5 py-0.5 font-mono text-[11px] text-ink-subtle">
                  {ev.ref}
                </span>
                {ev.claim}
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <SectionTitle>{t("cases.verdict.disclaimer")}</SectionTitle>
        <p className="text-[12px] leading-relaxed text-ink-subtle">
          {verdict.disclaimer}
        </p>
      </section>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-1.5 text-[11px] uppercase tracking-[0.08em] text-ink-faint">
      {children}
    </h4>
  );
}

/** Section title + a compact copy-to-clipboard affordance. Clinicians
 *  routinely paste recommendations into EHR notes, so every major
 *  verdict block gets one. */
function SectionRow({ title, copyText }: { title: string; copyText: string }) {
  return (
    <div className="mb-1.5 flex items-center justify-between gap-2">
      <SectionTitle>{title}</SectionTitle>
      <CopyButton text={copyText} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// ClassifyDropDialog — modal opened when the clinician drops ≥ 2 files on
// the Cases window. Renders the heuristic grouping (from
// `propose_case_grouping`) as editable patient cards with native HTML5
// drag/drop between cards, and offers two terminal actions:
//   • "Guardar como borradores" → `create_draft_cases` (no run)
//   • "Ejecutar comité (N)"     → `run_batch_cases` (creates + runs)
// ---------------------------------------------------------------------------

const DROP_MIME = "application/x-conclave-classify-file";

type DragPayload = { fromRow: number; fileIdx: number };

function ClassifyDropDialog({
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
  const [draggingFrom, setDraggingFrom] = useState<number | null>(null);
  const [openNoteIdx, setOpenNoteIdx] = useState<number | null>(null);
  // Deliberative toggle for the batch path. When ON, every case in the
  // batch runs through the 4-pass committee. Drafts ignore the toggle
  // (they're persisted-only) — promoting a draft later uses quick mode.
  const [deliberative, setDeliberative] = useState(false);

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
            ✕
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
                    isDragSource={draggingFrom === i}
                    isDropEligible={
                      draggingFrom !== null && draggingFrom !== i
                    }
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
                    onDragStart={() => setDraggingFrom(i)}
                    onDragEnd={() => setDraggingFrom(null)}
                    onDropFile={(p) => moveFile(p.fromRow, p.fileIdx, i)}
                  />
                ))}
                {draggingFrom !== null && (
                  <ClassifyNewCardDropTarget
                    onDropFile={(p) => moveFile(p.fromRow, p.fileIdx, "new")}
                    onDragEnd={() => setDraggingFrom(null)}
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
          <DeliberativeToggle
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
                {t("cases.classify_dialog_run_all", { count: rows.length })}
                {runDisabledReason ? ` ⚠` : ""}
              </Button>
            </span>
          </div>
        </footer>
      </div>
      {/* Touch onOpenCase to silence the unused-warning lint until we
          surface a per-card "Open" affordance in a follow-up. */}
      {false && <span onClick={() => onOpenCase("")} />}
    </div>
  );
}

function readDragPayload(e: React.DragEvent): DragPayload | null {
  try {
    const raw = e.dataTransfer.getData(DROP_MIME);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as DragPayload;
    if (
      typeof parsed.fromRow !== "number" ||
      typeof parsed.fileIdx !== "number"
    )
      return null;
    return parsed;
  } catch {
    return null;
  }
}

function ClassifyCard({
  row,
  index,
  isDragSource,
  isDropEligible,
  busy,
  noteOpen,
  onToggleNote,
  onLabelChange,
  onQuestionChange,
  onTextChange,
  onRemoveCase,
  onRemoveFile,
  onDragStart,
  onDragEnd,
  onDropFile,
}: {
  row: BatchCaseInput;
  index: number;
  isDragSource: boolean;
  /** True while a sibling card is being dragged. Used to subtly
   *  highlight this card as a valid drop target without forcing the
   *  user to hover first. */
  isDropEligible: boolean;
  busy: boolean;
  noteOpen: boolean;
  onToggleNote: () => void;
  onLabelChange: (v: string) => void;
  onQuestionChange: (v: string) => void;
  onTextChange: (v: string) => void;
  onRemoveCase: () => void;
  onRemoveFile: (fileIdx: number) => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onDropFile: (p: DragPayload) => void;
}) {
  const { t } = useTranslation();
  const [dragOver, setDragOver] = useState(false);

  return (
    <div
      onDragOver={(e) => {
        if (busy) return;
        const has = Array.from(e.dataTransfer.types).includes(DROP_MIME);
        if (!has) return;
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        setDragOver(false);
        if (busy) return;
        const payload = readDragPayload(e);
        if (!payload) return;
        e.preventDefault();
        onDropFile(payload);
        onDragEnd();
      }}
      className={cn(
        "rounded-lg border bg-bg p-3 transition",
        dragOver
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
            <span aria-hidden>✕</span>
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
            draggable={!busy}
            onDragStart={(e) => {
              if (busy) return;
              e.dataTransfer.setData(
                DROP_MIME,
                JSON.stringify({ fromRow: index, fileIdx }),
              );
              e.dataTransfer.effectAllowed = "move";
              onDragStart();
            }}
            onDragEnd={onDragEnd}
            className={cn(
              "group/chip flex max-w-full items-center gap-1.5 rounded-md border border-border-subtle bg-bg-subtle px-2 py-1 text-[11.5px] text-ink-dim transition",
              !busy && "cursor-grab hover:border-accent/40 hover:bg-bg hover:shadow-sm active:cursor-grabbing",
            )}
            title={busy ? path : t("cases.classify_dialog_chip_drag_hint", { path })}
          >
            <span aria-hidden className="select-none text-ink-faint group-hover/chip:text-accent">
              ⋮⋮
            </span>
            <ClassifyFileChip path={path} />
            {!busy && (
              <button
                type="button"
                onClick={() => onRemoveFile(fileIdx)}
                aria-label={t("cases.attachment_remove")}
                className="rounded p-0.5 text-ink-faint transition hover:bg-surface hover:text-ink"
              >
                ✕
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

function ClassifyNewCardDropTarget({
  onDropFile,
  onDragEnd,
}: {
  onDropFile: (p: DragPayload) => void;
  onDragEnd: () => void;
}) {
  const { t } = useTranslation();
  const [over, setOver] = useState(false);
  return (
    <div
      onDragOver={(e) => {
        const has = Array.from(e.dataTransfer.types).includes(DROP_MIME);
        if (!has) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        const payload = readDragPayload(e);
        if (!payload) return;
        e.preventDefault();
        onDropFile(payload);
        onDragEnd();
      }}
      className={cn(
        "flex items-center justify-center rounded-lg border border-dashed py-6 text-[12.5px] transition",
        over
          ? "border-accent bg-accent/5 text-accent"
          : "border-border-subtle text-ink-faint",
      )}
    >
      + {t("cases.classify_dialog_drop_new_card")}
    </div>
  );
}

function deriveLabelFromFile(path: string, fallbackIndex: number): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const stem = name.replace(/\.[^.]+$/, "").trim();
  return stem || `Paciente ${fallbackIndex}`;
}

// ---------------------------------------------------------------------------
// Deliberative mode — toggle, in-flight overlay, post-hoc trace accordion.
// ---------------------------------------------------------------------------

function DeliberativeToggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      aria-pressed={checked}
      className={cn(
        "group w-full rounded-lg border px-3 py-2.5 text-left transition focus:outline-none focus-visible:ring-conclave",
        checked
          ? "border-accent bg-accent/5"
          : "border-border-subtle bg-bg-subtle hover:border-border",
      )}
    >
      <div className="flex items-center gap-3">
        <span
          className={cn(
            "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition",
            checked ? "bg-accent" : "bg-border",
          )}
        >
          <span
            className={cn(
              "inline-block h-4 w-4 transform rounded-full bg-white transition",
              checked ? "translate-x-4" : "translate-x-0.5",
            )}
          />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-ink">
            {t("cases.deliberative_toggle_title")}
          </div>
          <div className="mt-0.5 text-[11.5px] text-ink-subtle">
            {t("cases.deliberative_toggle_subtitle")}
          </div>
        </div>
      </div>
    </button>
  );
}

// Warning banner shown when the currently-picked provider reports
// `available === false`. It doesn't disable the run button — the user
// might be reconnecting Ollama right this second — but it surfaces the
// fact so they don't burn an LLM call (or 11) before discovering it.
function ProviderOfflineBanner({
  providers,
  providerId,
}: {
  providers: ProviderInfo[];
  providerId: string;
}) {
  const { t } = useTranslation();
  const current = providers.find((p) => p.id === providerId);
  if (!current || current.available) return null;
  return (
    <div className="rounded-md border border-amber-400/40 bg-amber-400/10 px-3 py-2 text-[12px] text-amber-200">
      {t("cases.provider_offline_warning", { name: metaFor(current.id).name })}
    </div>
  );
}

const PHASE_ORDER: DeliberationPhase[] = [
  "briefing",
  "drafting",
  "redteam",
  "finalize",
];

type PhaseState = {
  status: "pending" | "active" | "done" | "failed";
  output?: string;
  error?: string;
  /** Wall-clock duration of this phase as reported by the backend.
   *  Present only on `done` / `failed` — pending/active rows omit it. */
  elapsedMs?: number;
};

function phaseIcon(phase: DeliberationPhase): string {
  switch (phase) {
    case "briefing":
      return "🩺";
    case "drafting":
      return "✍️";
    case "redteam":
      return "🛡️";
    case "finalize":
      return "📋";
  }
}

/**
 * In-flight overlay shown while a deliberative case is running. Listens
 * to the backend's `deliberation:progress` events and renders four
 * "committee seats" that pulse / fill in / mark ✓ as the LLM works
 * through each phase. Stays visible after the run finishes so the user
 * can review per-phase output and explicitly Close or jump to the
 * verdict — disappears only when the parent flips `deliberationActive`
 * back to `false`.
 */
function DeliberationOverlay({
  onDismiss,
}: {
  /** Called when the user clicks Close or presses Esc on the overlay
   *  AFTER the deliberation has finished. While the run is still in
   *  flight the overlay swallows Esc to avoid dropping a half-formed
   *  4-phase committee mid-call. */
  onDismiss?: () => void;
}) {
  const { t } = useTranslation();
  const [phases, setPhases] = useState<Record<DeliberationPhase, PhaseState>>(
    () => ({
      briefing: { status: "pending" },
      drafting: { status: "pending" },
      redteam: { status: "pending" },
      finalize: { status: "pending" },
    }),
  );
  const [expanded, setExpanded] = useState<DeliberationPhase | null>(null);
  /** Flipped on `done` so the overlay shifts from "running" copy to the
   *  "deliberation complete — view verdict / close" CTA pair. */
  const [done, setDone] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    (async () => {
      unlisten = await listen<DeliberationEvent>(
        "deliberation:progress",
        (msg) => {
          if (cancelled) return;
          const ev = msg.payload;
          if (ev.kind === "done") {
            setDone(true);
            return;
          }
          setPhases((prev) => {
            const next = { ...prev };
            if (ev.kind === "phase_started") {
              next[ev.phase] = { status: "active" };
              setExpanded(ev.phase);
            } else if (ev.kind === "phase_completed") {
              next[ev.phase] = {
                status: "done",
                output: ev.output,
                elapsedMs: ev.elapsed_ms,
              };
            } else if (ev.kind === "phase_failed") {
              next[ev.phase] = {
                status: "failed",
                error: ev.error,
                elapsedMs: ev.elapsed_ms,
              };
            }
            return next;
          });
        },
      );
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Esc closes the overlay ONLY after the deliberation is done — we
  // don't want a stray keypress to abandon a $0.30 / 4-phase run.
  useEffect(() => {
    if (!done) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss?.();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [done, onDismiss]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("cases.deliberation_overlay_title")}
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/55 backdrop-blur-[2px] px-4 py-10"
    >
      <div className="flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-border bg-bg-elevated shadow-soft">
        <header className="border-b border-border-subtle px-5 py-4">
          <h2 className="text-[15px] font-semibold text-ink">
            {done
              ? t("cases.deliberation_overlay_done_title")
              : t("cases.deliberation_overlay_title")}
          </h2>
          <p className="mt-0.5 text-[12.5px] text-ink-subtle">
            {done
              ? t("cases.deliberation_overlay_done_subtitle")
              : t("cases.deliberation_overlay_subtitle")}
          </p>
        </header>
        <div className="flex-1 space-y-3 overflow-y-auto px-5 py-4">
          {PHASE_ORDER.map((phase, i) => (
            <DeliberationPhaseRow
              key={phase}
              phase={phase}
              index={i}
              state={phases[phase]}
              expanded={expanded === phase}
              onToggle={() =>
                setExpanded(expanded === phase ? null : phase)
              }
            />
          ))}
        </div>
        {done && onDismiss && (
          <footer className="flex justify-end gap-2 border-t border-border-subtle px-5 py-3">
            <Button size="sm" variant="ghost" onClick={onDismiss}>
              {t("cases.deliberation_overlay_close")}
            </Button>
          </footer>
        )}
      </div>
    </div>
  );
}

function DeliberationPhaseRow({
  phase,
  index,
  state,
  expanded,
  onToggle,
}: {
  phase: DeliberationPhase;
  index: number;
  state: PhaseState;
  expanded: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const badge = (() => {
    switch (state.status) {
      case "pending":
        return (
          <span className="rounded bg-surface px-2 py-0.5 text-[10.5px] uppercase tracking-wide text-ink-faint">
            {t("cases.phase_status_pending")}
          </span>
        );
      case "active":
        return (
          <span className="flex items-center gap-1 rounded bg-accent/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-accent">
            <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
            {t("cases.phase_status_active")}
          </span>
        );
      case "done":
        return (
          <span className="flex items-center gap-1.5 rounded bg-ok/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-ok">
            <span>✓ {t("cases.phase_status_done")}</span>
            {state.elapsedMs !== undefined && (
              <span className="font-mono text-[9.5px] text-ok/80">
                {formatElapsed(state.elapsedMs)}
              </span>
            )}
          </span>
        );
      case "failed":
        return (
          <span className="flex items-center gap-1.5 rounded bg-danger/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-danger">
            <span>✗ {t("cases.phase_status_failed")}</span>
            {state.elapsedMs !== undefined && (
              <span className="font-mono text-[9.5px] text-danger/80">
                {formatElapsed(state.elapsedMs)}
              </span>
            )}
          </span>
        );
    }
  })();

  return (
    <div
      className={cn(
        "rounded-lg border transition",
        state.status === "active"
          ? "border-accent/60 bg-accent/5"
          : "border-border-subtle bg-bg",
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-3 px-3 py-2.5 text-left focus:outline-none focus-visible:ring-conclave"
      >
        <span className="font-mono text-[11px] text-ink-faint">
          {index + 1}/4
        </span>
        <span className="text-base">{phaseIcon(phase)}</span>
        <div className="min-w-0 flex-1">
          <div className="text-[13.5px] font-medium text-ink">
            {t(`cases.deliberation_phase.${phase}_title`)}
          </div>
          <div className="mt-0.5 text-[11.5px] text-ink-subtle">
            {t(`cases.deliberation_phase.${phase}_subtitle`)}
          </div>
        </div>
        {badge}
      </button>
      {expanded && (state.output || state.error) && (
        <div className="border-t border-border-subtle px-3 py-3">
          {state.error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[12px] text-danger">
              {state.error}
            </div>
          )}
          {state.output && (
            <div className="prose-conclave max-h-[360px] overflow-auto text-[12.5px]">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {state.output}
              </ReactMarkdown>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Post-hoc trace shown inside ShowCase. Fetches the persisted
 * deliberation trace for the verdict; renders nothing for quick-mode
 * cases (the IPC returns `null`).
 */
function DeliberationTraceAccordion({
  workspaceId,
  verdictId,
}: {
  workspaceId: string;
  verdictId: string;
}) {
  const { t } = useTranslation();
  const [trace, setTrace] = useState<DeliberationTrace | null | undefined>(
    undefined,
  );
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<DeliberationPhase | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await ipc.getDeliberationTrace(workspaceId, verdictId);
        if (!cancelled) setTrace(result);
      } catch {
        if (!cancelled) setTrace(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [workspaceId, verdictId]);

  if (trace === undefined) return null; // still loading
  if (trace === null) return null; // not a deliberative case

  const phases: { phase: DeliberationPhase; output: string | null }[] = [
    { phase: "briefing", output: trace.briefing_output },
    { phase: "drafting", output: trace.drafting_output },
    { phase: "redteam", output: trace.redteam_output },
  ];
  const totalTokens = trace.total_input_tokens + trace.total_output_tokens;
  const seconds = Math.round(trace.duration_ms / 100) / 10;

  return (
    <Card>
      <CardHeader
        title={t("cases.deliberation_trace_title")}
        subtitle={t("cases.deliberation_trace_subtitle", {
          tokens: totalTokens,
          duration: seconds,
        })}
        right={
          <Button size="sm" variant="ghost" onClick={() => setOpen(!open)}>
            {open
              ? t("cases.deliberation_trace_collapse")
              : t("cases.deliberation_trace_expand")}
          </Button>
        }
      />
      {open && (
        <CardBody className="space-y-2">
          {trace.vision_used && (
            <div className="rounded-md border border-violet-400/40 bg-violet-400/5 px-3 py-2 text-[12px] text-violet-200">
              {t("cases.deliberation_vision_used")}
            </div>
          )}
          {phases.map(({ phase, output }) => (
            <div
              key={phase}
              className="rounded-lg border border-border-subtle bg-bg"
            >
              <button
                type="button"
                onClick={() =>
                  setExpanded(expanded === phase ? null : phase)
                }
                className="flex w-full items-center gap-3 px-3 py-2 text-left focus:outline-none focus-visible:ring-conclave"
              >
                <span className="text-base">{phaseIcon(phase)}</span>
                <span className="flex-1 text-[13px] font-medium text-ink">
                  {t(`cases.deliberation_phase.${phase}_title`)}
                </span>
                <span className="text-[11px] text-ink-faint">
                  {expanded === phase ? "▾" : "▸"}
                </span>
              </button>
              {expanded === phase && output && (
                <div className="border-t border-border-subtle px-3 py-3">
                  <div className="prose-conclave max-h-[400px] overflow-auto text-[12.5px]">
                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
                      {output}
                    </ReactMarkdown>
                  </div>
                </div>
              )}
              {expanded === phase && !output && (
                <div className="border-t border-border-subtle px-3 py-2 text-[12px] text-ink-faint">
                  {t("cases.deliberation_phase_empty")}
                </div>
              )}
            </div>
          ))}
        </CardBody>
      )}
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Helpers — banner, per-row phase chip, copy button, skeleton.
// ---------------------------------------------------------------------------

/** Batch banner: shows live elapsed time + an ETA derived from the
 *  first completed case. The user gets a sense of pace AND a "how long
 *  is left" estimate without us touching the deliberation pipeline. */
function BatchProgressBanner({
  done,
  total,
  startedAtMs,
  firstCaseMs,
  tickMs,
  onCancelAll,
}: {
  done: number;
  total: number;
  startedAtMs: number | null;
  firstCaseMs: number | null;
  /** Time tick from CasesPage — we ignore the value (it just forces
   *  a re-render). Without it the elapsed chip would stay frozen until
   *  another React update arrived. */
  tickMs: number;
  onCancelAll: () => void;
}) {
  void tickMs;
  const { t } = useTranslation();
  const elapsedMs = startedAtMs === null ? 0 : Math.max(0, Date.now() - startedAtMs);
  // ETA heuristic: each remaining case takes ~ firstCaseMs. The first
  // case usually has cold-cache penalties (provider auth, embedder
  // warmup) so this overestimates a bit — fine, better than nothing.
  const remaining = Math.max(0, total - done);
  const etaMs = firstCaseMs !== null ? remaining * firstCaseMs : null;

  return (
    <div className="flex flex-wrap items-center gap-3 rounded-md border border-accent/40 bg-accent/5 px-3 py-2 text-[13px] text-accent">
      <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
      <span>{t("cases.batch_progress_banner", { done, total })}</span>
      {startedAtMs !== null && (
        <span className="rounded bg-accent/10 px-2 py-0.5 font-mono text-[11.5px]">
          {t("cases.batch_progress_elapsed", { elapsed: formatElapsed(elapsedMs) })}
        </span>
      )}
      {etaMs !== null && remaining > 0 && (
        <span className="rounded bg-accent/10 px-2 py-0.5 font-mono text-[11.5px]">
          {t("cases.batch_progress_eta", { eta: formatElapsed(etaMs) })}
        </span>
      )}
      <button
        type="button"
        onClick={onCancelAll}
        className="ml-auto rounded-md border border-accent/30 px-2 py-0.5 text-[11.5px] text-accent transition hover:bg-accent/10 focus:outline-none focus-visible:ring-conclave"
      >
        {t("cases.batch_cancel_all")}
      </button>
    </div>
  );
}

/** Compact chip rendered next to a running case row: shows the current
 *  phase name + ticking elapsed time. Quick-mode runs never produce
 *  these — they just animate the regular `running…` status badge. */
function PhaseRunningChip({
  phase,
  tickMs,
}: {
  phase: LiveCasePhase;
  tickMs: number;
}) {
  void tickMs;
  const { t } = useTranslation();
  const label = t(`cases.phase_short.${phase.phase}`);
  const elapsedMs =
    phase.status === "active" ? Math.max(0, Date.now() - phase.startedAtMs) : 0;
  const colour =
    phase.status === "active"
      ? "bg-accent/10 text-accent"
      : phase.status === "failed"
        ? "bg-danger/10 text-danger"
        : "bg-ok/10 text-ok";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[10.5px]",
        colour,
      )}
      title={t("cases.row_running_phase", { phase: label })}
    >
      <span>{label}</span>
      {phase.status === "active" && elapsedMs > 0 && (
        <span>· {formatElapsed(elapsedMs)}</span>
      )}
    </span>
  );
}

/** Small button that copies `text` to the clipboard and flashes a
 *  brief "copied" confirmation. Reused across every verdict section. */
function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    };
  }, []);
  return (
    <button
      type="button"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
        } catch {
          // Some Tauri contexts disallow clipboard access; the user
          // can still select and copy manually. Surface silently.
          return;
        }
        setCopied(true);
        if (timerRef.current !== null) window.clearTimeout(timerRef.current);
        timerRef.current = window.setTimeout(() => setCopied(false), 1500);
      }}
      title={t("cases.copy_field")}
      aria-label={t("cases.copy_field")}
      className={cn(
        "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10.5px]",
        "text-ink-faint transition hover:bg-surface hover:text-ink",
        "focus:outline-none focus-visible:ring-conclave",
        copied && "text-ok",
      )}
    >
      {copied ? (
        <>
          <IconCheck aria-hidden="true" size={12} stroke={1.8} />
          <span>{t("cases.copied_toast")}</span>
        </>
      ) : (
        <>
          <IconCopy aria-hidden="true" size={12} stroke={1.6} />
          <span>{t("cases.copy_field")}</span>
        </>
      )}
    </button>
  );
}

/** Skeleton placeholder for the case-detail view. Shown during the
 *  brief optimistic gap between clicking a row and the `showCase` IPC
 *  resolving — keeps the user oriented instead of a flash of blank. */
function ShowCaseSkeleton({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="mx-auto w-full max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onBack}>
          {t("cases.back")}
        </Button>
      </div>
      <Card>
        <CardHeader
          title={t("cases.show_case_loading")}
          subtitle={" "}
        />
        <CardBody className="space-y-4">
          <div className="h-3 w-2/3 animate-pulse rounded bg-surface" />
          <div className="h-3 w-1/2 animate-pulse rounded bg-surface" />
          <div className="h-24 w-full animate-pulse rounded bg-surface" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-surface" />
          <div className="h-3 w-1/3 animate-pulse rounded bg-surface" />
          <div className="h-16 w-full animate-pulse rounded bg-surface" />
        </CardBody>
      </Card>
    </div>
  );
}
