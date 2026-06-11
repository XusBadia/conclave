// Presentational leaf components for the Cases route: status badge,
// batch/PDF progress + result banners, the per-row phase chip, the copy
// button and the detail-view skeleton. Pure props in, JSX out — no IPC,
// no page state.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconCheck, IconCopy, IconX } from "@tabler/icons-react";

import type { BatchExportResult } from "../../pdf/exportCasesBatch";
import { Button } from "../../components/Button";
import { Card, CardBody, CardHeader } from "../../components/Card";
import { cn } from "../../lib/cn";
import type { CaseRecord } from "../../lib/ipc";
import { formatElapsed, type LiveCasePhase } from "./helpers";

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
export function StatusBadge({
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
  if (status === "finalized" || status === "finalized_legacy") {
    return (
      <span className="rounded bg-ok/15 px-2 py-0.5 text-[11px] font-medium text-ok">
        {t("cases.status.finalized")}
      </span>
    );
  }
  if (status === "review_ready") {
    return (
      <span className="rounded bg-accent/15 px-2 py-0.5 text-[11px] font-medium text-accent">
        {t("cases.status.review_ready")}
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

export function BatchProgressBanner({
  done,
  total,
  startedAtMs,
  firstCaseMs,
  tickMs,
  cancelling,
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
  /** True while a batch-wide cancel is in flight. */
  cancelling: boolean;
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
        disabled={cancelling}
        className={cn(
          "ml-auto rounded-md border border-accent/30 px-2 py-0.5 text-[11.5px] text-accent transition hover:bg-accent/10 focus:outline-none focus-visible:ring-conclave",
          cancelling && "cursor-default opacity-60 hover:bg-transparent",
        )}
      >
        {t(cancelling ? "cases.batch_cancelling" : "cases.batch_cancel_all")}
      </button>
    </div>
  );
}

/** Progress banner for a multi-case PDF export. Mirrors the deliberation
 *  BatchProgressBanner styling so the two read as the same family. The Cancel
 *  button aborts the run between cases (PDFs already written stay on disk). */
export function PdfExportBanner({
  done,
  total,
  onCancel,
}: {
  done: number;
  total: number;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-md border border-accent/40 bg-accent/5 px-3 py-2 text-[13px] text-accent">
      <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-accent" />
      <span>{t("cases.batch_export_progress", { done, total })}</span>
      <button
        type="button"
        onClick={onCancel}
        className="ml-auto rounded-md border border-accent/30 px-2 py-0.5 text-[11.5px] text-accent transition hover:bg-accent/10 focus:outline-none focus-visible:ring-conclave"
      >
        {t("common.cancel")}
      </button>
    </div>
  );
}

/** Dismissible summary shown after a multi-case PDF export settles: how many
 *  PDFs landed where, plus any skipped (verdict-less) or aborted note. Green
 *  when something was written, amber when nothing was. */
export function PdfExportResultBanner({
  result,
  onDismiss,
}: {
  result: BatchExportResult;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  const none = result.saved === 0;
  const parts: string[] = [];
  if (none) {
    parts.push(t("cases.batch_export_none"));
  } else {
    parts.push(
      t(
        result.saved === 1
          ? "cases.batch_export_done"
          : "cases.batch_export_done_plural",
        { count: result.saved, dir: result.dir ?? "" },
      ),
    );
  }
  if (result.skipped.length > 0) {
    parts.push(
      t(
        result.skipped.length === 1
          ? "cases.batch_export_skipped"
          : "cases.batch_export_skipped_plural",
        { count: result.skipped.length },
      ),
    );
  }
  if (result.aborted) {
    parts.push(t("cases.batch_export_aborted"));
  }
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-3 rounded-md border px-3 py-2 text-[13px]",
        none
          ? "border-warn/40 bg-warn/10 text-warn"
          : "border-ok/30 bg-ok/5 text-ok",
      )}
    >
      <span className="break-words">{parts.join(" · ")}</span>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={t("common.dismiss")}
        className={cn(
          "shrink-0 rounded p-0.5 transition",
          none
            ? "text-warn/70 hover:bg-warn/10 hover:text-warn"
            : "text-ok/70 hover:bg-ok/10 hover:text-ok",
        )}
      >
        <IconX size={14} stroke={1.7} aria-hidden />
      </button>
    </div>
  );
}

/** Compact chip rendered next to a running case row: shows the current
 *  phase name + ticking elapsed time. Quick-mode runs never produce
 *  these — they just animate the regular `running…` status badge. */
export function PhaseRunningChip({
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
export function CopyButton({ text }: { text: string }) {
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
export function ShowCaseSkeleton({ onBack }: { onBack: () => void }) {
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
