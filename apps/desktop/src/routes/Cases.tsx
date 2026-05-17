import { useEffect, useMemo, useState } from "react";
import type { TFunction } from "i18next";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input, Textarea } from "../components/Field";
import { Sheet } from "../components/Sheet";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type CaseAttachment,
  type CaseDetail,
  type CaseRecord,
  type ProviderInfo,
  type Verdict,
  type Workspace,
} from "../lib/ipc";
import { metaFor } from "../lib/providers";

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
  const [unsupportedDropError, setUnsupportedDropError] = useState<string | null>(
    null,
  );

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
            setPendingDrop((prev) => dedupeAttachments(prev, accepted));
            // Anything dropped while on the list lands in the new-case
            // form. From there NewCase merges it with whatever state it
            // already has so successive drops keep stacking.
            setView((v) => (v === "list" || v === "show" ? "new" : v));
            setUnsupportedDropError(null);
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
    return (
      <NewCase
        workspace={workspace}
        onCancel={() => {
          setView("list");
          setPendingDrop([]);
        }}
        onGoToSettings={onGoToSettings}
        incomingAttachments={pendingDrop}
        onIncomingConsumed={() => setPendingDrop([])}
        onDone={async (id) => {
          setPendingDrop([]);
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
                      setView("show");
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
                      <span
                        className={
                          c.status === "completed"
                            ? "rounded bg-ok/15 px-2 py-0.5 text-[11px] font-medium text-ok"
                            : "rounded bg-danger/15 px-2 py-0.5 text-[11px] font-medium text-danger"
                        }
                      >
                        {t(`cases.status.${c.status}`)}
                      </span>
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
}: {
  workspace: Workspace;
  onCancel: () => void;
  onDone: (caseId: string) => void;
  onGoToSettings?: () => void;
  incomingAttachments?: PendingAttachment[];
  onIncomingConsumed?: () => void;
}) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [text, setText] = useState("");
  const [question, setQuestion] = useState(t("cases.default_question"));
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

  const usable = useMemo(() => usableProviders(providers), [providers]);

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
    if (!text.trim() && attachments.length === 0) return;
    if (!providerId) {
      setError(t("cases.no_provider_configured"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const resp = await ipc.runCase({
        workspace_id: workspace.id,
        text,
        question,
        provider_id: providerId,
        attached_file_paths: attachments.map((a) => a.path),
      });
      onDone(resp.case.id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
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
          <NewCaseAttachments
            attachments={attachments}
            onBrowse={pickFiles}
            onRemove={removeAttachment}
          />
          <Field label={t("cases.field_question")}>
            <Input value={question} onChange={(e) => setQuestion(e.target.value)} />
          </Field>
          <ProviderField
            providers={usable}
            providerId={providerId}
            onChange={setProviderId}
            onGoToSettings={onGoToSettings}
          />
          <div className="flex gap-2 pt-1">
            <Button onClick={previewDeident} disabled={!text.trim()}>
              {t("cases.preview_button")}
            </Button>
            <Button
              variant="primary"
              onClick={run}
              loading={busy}
              disabled={(!text.trim() && attachments.length === 0) || !providerId}
            >
              {t("cases.run_button")}
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
