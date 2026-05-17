import { useEffect, useMemo, useState } from "react";
import type { TFunction } from "i18next";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

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
import { isClinicalEligible, metaFor } from "../lib/providers";

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

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setCases(await ipc.listCases(workspace.id, 50));
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
    setPendingDrop([]);
    setUnsupportedDropError(null);
    setClassifyDialog(null);
    setRunningCaseIds(new Set());
    setBatchTotal(null);
    setBatchDone(0);
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
          setBatchDone((d) => d + 1);
          void refresh();
        } else if (ev.kind === "case_failed") {
          setBatchDone((d) => d + 1);
          void refresh();
        } else if (ev.kind === "case_cancelled") {
          setBatchDone((d) => d + 1);
        } else if (ev.kind === "batch_done") {
          setBatchTotal(null);
          setBatchDone(0);
          void refresh();
        }
      });
    })();
    return () => {
      cancelled = true;
      unlistenDrafted?.();
      unlistenBatch?.();
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

  if (view === "show" && selected) {
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
      {batchTotal !== null && batchTotal > 0 && (
        <div className="flex items-center gap-2 rounded-md border border-accent/40 bg-accent/5 px-3 py-2 text-[13px] text-accent">
          <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
          <span>
            {t("cases.batch_progress_banner", {
              done: batchDone,
              total: batchTotal,
            })}
          </span>
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
              return (
                <li key={row.key}>
                  <button
                    type="button"
                    onClick={async () => {
                      if (selectionMode) {
                        toggleSelected(c.id);
                        return;
                      }
                      const det = await ipc.showCase(workspace.id, c.id);
                      setSelected(det);
                      // Drafts have no verdict yet — open NewCase
                      // pre-filled so the clinician can add clinical
                      // context and run them. Completed/Failed go to
                      // ShowCase as before.
                      setView(
                        det && det.case.status === "draft" ? "new" : "show",
                      );
                    }}
                    className={cn(
                      "block w-full px-5 py-4 text-left transition focus:outline-none focus-visible:bg-surface",
                      isSelected ? "bg-accent/5 hover:bg-accent/10" : "hover:bg-surface",
                    )}
                  >
                    <div className="flex items-center gap-3">
                      {selectionMode && (
                        <input
                          type="checkbox"
                          checked={isSelected}
                          readOnly
                          aria-label={c.question || c.id}
                          className="h-4 w-4 shrink-0 accent-accent"
                          tabIndex={-1}
                        />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[14px] font-medium text-ink">
                          {c.question || t("cases.no_question")}
                        </div>
                        <div className="mt-0.5 truncate text-[12px] text-ink-faint">
                          <span className="font-mono">{c.id}</span> ·{" "}
                          {new Date(c.case_date).toLocaleString()}
                        </div>
                      </div>
                      <StatusBadge
                        status={c.status}
                        running={runningCaseIds.has(c.id)}
                      />
                    </div>
                  </button>
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

      {classifyDialog && (
        <ClassifyDropDialog
          workspace={workspace}
          initialProposal={classifyDialog.proposal}
          loading={classifyDialog.loading}
          onClose={() => setClassifyDialog(null)}
          onCommitted={async () => {
            setClassifyDialog(null);
            await refresh();
          }}
          onOpenCase={async (caseId) => {
            setClassifyDialog(null);
            const det = await ipc.showCase(workspace.id, caseId);
            setSelected(det);
            setView(det && det.case.status === "draft" ? "new" : "show");
          }}
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

  useEffect(() => {
    (async () => {
      const ps = await ipc.listProviders();
      setProviders(ps);
      const first = ps.find((p) => p.configured || p.id === "ollama");
      if (first) setProviderId(first.id);
    })();
  }, []);

  // Merge any incoming page-level drag-drop payload with our local
  // attachments. Cleared in the parent after we've integrated it.
  useEffect(() => {
    if (!incomingAttachments || incomingAttachments.length === 0) return;
    setAttachments((prev) => dedupeAttachments(prev, incomingAttachments));
    onIncomingConsumed?.();
  }, [incomingAttachments, onIncomingConsumed]);

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
        onDone(resp.case.id);
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
    } finally {
      setBusy(false);
      setDeliberationActive(false);
    }
  };

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
            <svg
              aria-hidden="true"
              viewBox="0 0 16 16"
              className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ok"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="3" y="7" width="10" height="6.5" rx="1.2" />
              <path d="M5.2 7V4.8a2.8 2.8 0 0 1 5.6 0V7" />
            </svg>
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
      {deliberationActive && <DeliberationOverlay />}
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
  detail: CaseDetail;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const feedback = async (kind: "accept" | "modify" | "reject") => {
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
          title={detail.case.question || t("cases.no_question")}
          subtitle={`${detail.case.id} · ${new Date(detail.case.created_at).toLocaleString()}`}
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
          {detail.verdict ? (
            <VerdictRenderer verdict={detail.verdict} />
          ) : (
            <p className="text-[13px] text-ink-subtle">
              {t("cases.no_verdict")}
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

  return (
    <div className="space-y-6">
      <section>
        <SectionTitle>{t("cases.verdict.case_summary")}</SectionTitle>
        <p>{verdict.case_summary}</p>
      </section>

      {verdict.key_clinical_data.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.key_clinical_data")}</SectionTitle>
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
        <SectionTitle>{t("cases.verdict.primary_recommendation")}</SectionTitle>
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
          <SectionTitle>{t("cases.verdict.alternatives")}</SectionTitle>
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
        <SectionTitle>{t("cases.verdict.certainty")}</SectionTitle>
        <div className={`text-[14px] font-semibold ${certaintyColor}`}>
          {verdict.certainty_level.toUpperCase()}
        </div>
        <p className="mt-1">{verdict.certainty_justification}</p>
      </section>

      {verdict.red_flags.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.red_flags")}</SectionTitle>
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
          <SectionTitle>{t("cases.verdict.follow_up_triggers")}</SectionTitle>
          <ul className="list-inside list-disc space-y-1 text-[13px] text-ink-dim">
            {verdict.follow_up_triggers.map((tr, i) => (
              <li key={i}>{tr}</li>
            ))}
          </ul>
        </section>
      )}

      {verdict.applied_evidence.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.applied_evidence")}</SectionTitle>
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
  onGoToSettings,
}: {
  workspace: Workspace;
  initialProposal: BatchCaseInput[];
  loading: boolean;
  onClose: () => void;
  onCommitted: () => void;
  onOpenCase: (caseId: string) => void;
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
      const first = ps.find((p) => p.configured || p.id === "ollama");
      if (first) setProviderId(first.id);
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
    setRows((prev) => prev.filter((_, idx) => idx !== i));
  };

  const removeFileFromRow = (rowIdx: number, fileIdx: number) => {
    setRows((prev) =>
      prev
        .map((r, idx) =>
          idx === rowIdx
            ? {
                ...r,
                attached_file_paths: r.attached_file_paths.filter(
                  (_, i) => i !== fileIdx,
                ),
              }
            : r,
        )
        .filter(
          (r) =>
            r.attached_file_paths.length > 0 || r.text.trim().length > 0,
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

  const runAll = () => {
    if (rows.length === 0) return;
    if (!providerId) {
      setError(t("cases.no_provider_configured"));
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
        // We won't see this directly (the dialog is gone) but log it
        // so the failure isn't silent. The frontend listener will also
        // surface per-case failures.
        // eslint-disable-next-line no-console
        console.error("batch run failed:", e);
      });
    onCommitted();
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("cases.classify_dialog_title")}
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/45 backdrop-blur-[2px] p-4"
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
              </div>
              <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                {rows.map((row, i) => (
                  <ClassifyCard
                    key={`${row.patient_label}-${i}`}
                    row={row}
                    index={i}
                    isDragSource={draggingFrom === i}
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
            <Button
              size="sm"
              variant="primary"
              onClick={runAll}
              loading={busy}
              disabled={busy || rows.length === 0 || !providerId}
            >
              {t("cases.classify_dialog_run_all", { count: rows.length })}
            </Button>
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
            aria-label={t("classify_dialog_close")}
            className="rounded p-1 text-ink-faint transition hover:bg-surface hover:text-ink"
          >
            ✕
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
            className="flex max-w-full items-center gap-1.5 rounded-md border border-border-subtle bg-bg-subtle px-2 py-1 text-[11.5px] text-ink-dim transition hover:border-border"
            title={path}
          >
            <span aria-hidden className="cursor-grab select-none text-ink-faint">
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
 * through each phase. Disappears when the parent flips
 * `deliberationActive` back to `false`.
 */
function DeliberationOverlay() {
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

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    (async () => {
      unlisten = await listen<DeliberationEvent>(
        "deliberation:progress",
        (msg) => {
          if (cancelled) return;
          const ev = msg.payload;
          setPhases((prev) => {
            const next = { ...prev };
            if (ev.kind === "phase_started") {
              next[ev.phase] = { status: "active" };
              setExpanded(ev.phase);
            } else if (ev.kind === "phase_completed") {
              next[ev.phase] = { status: "done", output: ev.output };
            } else if (ev.kind === "phase_failed") {
              next[ev.phase] = { status: "failed", error: ev.error };
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

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("cases.deliberation_overlay_title")}
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/55 backdrop-blur-[2px] p-4"
    >
      <div className="flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-border bg-bg-elevated shadow-soft">
        <header className="border-b border-border-subtle px-5 py-4">
          <h2 className="text-[15px] font-semibold text-ink">
            {t("cases.deliberation_overlay_title")}
          </h2>
          <p className="mt-0.5 text-[12.5px] text-ink-subtle">
            {t("cases.deliberation_overlay_subtitle")}
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
          <span className="rounded bg-ok/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-ok">
            ✓ {t("cases.phase_status_done")}
          </span>
        );
      case "failed":
        return (
          <span className="rounded bg-danger/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-danger">
            ✗ {t("cases.phase_status_failed")}
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
