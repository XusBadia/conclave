import type { CaseDetail } from "../lib/ipc";

const INVALID_FILENAME_CHARS = /[\\/:*?"<>|\x00-\x1f]/g;
const WHITESPACE_RUN = /\s+/g;

/** Sanitises a string so it is safe to use as a filename on macOS, Windows,
 *  and Linux. Strips control chars and the reserved set `\ / : * ? " < > |`,
 *  collapses runs of whitespace and surrounding punctuation, and trims. */
function sanitise(input: string): string {
  return input
    .replace(INVALID_FILENAME_CHARS, " ")
    .replace(WHITESPACE_RUN, " ")
    .replace(/[._-]+$|^[._-]+/g, "")
    .trim();
}

/** Formats an ISO-8601 / RFC3339 timestamp as `YYYY-MM-DD`. Falls back to the
 *  caller's "today" if the timestamp can't be parsed (older draft cases). */
function formatDate(rfc3339: string): string {
  const parsed = new Date(rfc3339);
  if (Number.isNaN(parsed.getTime())) {
    return new Date().toISOString().slice(0, 10);
  }
  return parsed.toISOString().slice(0, 10);
}

/** Builds the default filename suggested by the save dialog when exporting a
 *  case verdict to PDF. Shape: `<prefix>_<patient-or-id>_<YYYY-MM-DD>.pdf`. */
export function buildPdfFilename(detail: CaseDetail, prefix = "ConclaveMD"): string {
  const { patient_label, id, case_date, created_at } = detail.case;
  const label =
    sanitise(patient_label || "").slice(0, 60) ||
    id.slice(0, 8);
  const date = formatDate(case_date || created_at);
  const parts = [sanitise(prefix), label, date].filter(Boolean);
  return `${parts.join("_").replace(/\s+/g, "_")}.pdf`;
}
