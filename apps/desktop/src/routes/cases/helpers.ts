// Pure helpers for the Cases route: attachment intake, list
// grouping/sorting buckets, datetime-local conversion and compact
// durations. No React, no Tauri — keep it that way so the unit tests
// stay trivial.

import type { TFunction } from "i18next";

import type { DeliberationPhase } from "../../lib/ipc";

// Extensions we accept when the user drops or picks files for a case.
// Mirrors `apps/desktop/src-tauri/src/batch.rs::ATTACHMENT_EXTS` so the
// frontend filter and the backend extractor agree on what counts.
export const SUPPORTED_ATTACHMENT_EXTS = [
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

export type PendingAttachment = {
  path: string;
  name: string;
  ext: string;
  isImage: boolean;
};

export function attachmentFromPath(path: string): PendingAttachment | null {
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

export function dedupeAttachments(
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

export function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

export function attachmentBadgeColor(extOrType: string): string {
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

/** Detect labels that are still the filename-stem fallback (e.g.
 *  "CR-IA-007", "case_recto_bajo_alto_riesgo") rather than a proper
 *  patient summary from Apple Intelligence ("Mujer 67, recto bajo T3N1"
 *  — sentence-shaped, has spaces and commas).
 *
 *  Heuristic: empty, or zero spaces, or contains underscores. AI
 *  summaries always have at least one space and never use underscores
 *  in our prompt template. */
export function isFallbackLabel(label: string | null | undefined): boolean {
  if (!label) return true;
  const trimmed = label.trim();
  if (trimmed.length === 0) return true;
  // Filename stems and "CR-IA-XXX" codes have no spaces.
  if (!/\s/.test(trimmed)) return true;
  // Any underscore means it survived from a file name.
  if (trimmed.includes("_")) return true;
  return false;
}

export type SortBy = "date_desc" | "date_asc" | "question_az" | "status";
export type GroupBy = "off" | "day" | "week" | "month";

// Anchor a date to the start of its bucket (used both as map key and as
// the value we feed into bucketLabel — so the displayed name aligns with
// the rows it groups).
export function bucketAnchor(iso: string, mode: GroupBy): Date {
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

export function bucketKey(iso: string, mode: GroupBy): string {
  if (mode === "off") return "all";
  const d = bucketAnchor(iso, mode);
  if (mode === "month") return `${d.getFullYear()}-${d.getMonth() + 1}`;
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

export function bucketLabel(
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
export function isoToLocalInput(iso: string): string {
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}`;
}

export function localInputToIso(local: string): string {
  // The Date constructor interprets `YYYY-MM-DDTHH:mm` as local time.
  return new Date(local).toISOString();
}

/** Format an elapsed millisecond duration as `0:42` / `12:07` for
 *  short runs, and `1h 03m` once we cross the hour boundary. Tuned for
 *  the batch banner / per-row chip — they want compactness over
 *  precision. */
export function formatElapsed(ms: number): string {
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
export type LiveCasePhase = {
  phase: DeliberationPhase;
  /** ms-since-page-load when this phase started; used to render the
   *  ticking elapsed chip without a per-second re-render of the whole
   *  list (the LiveTicker child reads it via state). */
  startedAtMs: number;
  status: "active" | "done" | "failed";
};
