// The batch/live-progress state machine for the Cases page, extracted
// verbatim from CasesPage. Owns every piece of state driven by the
// backend's `case:drafted`, `batch:progress` and `deliberation:progress`
// events, plus the 1s tick that keeps elapsed chips moving.
//
// Refreshing the cases list stays a page concern — the hook only calls
// the three callbacks. They are captured at listener-bind time (once per
// workspace), mirroring the original effect's closure semantics exactly.

import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  ipc,
  type BatchEvent,
  type CaseDraftedEvent,
  type DeliberationEvent,
} from "../../lib/ipc";
import type { LiveCasePhase } from "./helpers";

export type RowBusyAction = "retry" | "cancel";

export function useBatchProgress({
  workspaceId,
  onRefresh,
  onScheduleRefresh,
  onRefreshNow,
}: {
  workspaceId: string;
  /** Fire-and-forget full list refresh (new draft appeared / case started). */
  onRefresh: () => void;
  /** Coalesced trailing refresh — per-case terminal events arrive in bursts. */
  onScheduleRefresh: () => void;
  /** Drop any pending coalesced refresh and refresh immediately (batch_done). */
  onRefreshNow: () => void;
}) {
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
  /** True while a batch-wide cancel is in flight, so the banner button
   *  reads "Cancelling batch…" and disables. Cleared on `batch_done`. */
  const [batchCancelling, setBatchCancelling] = useState(false);
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
  const [rowBusy, setRowBusy] = useState<Map<string, RowBusyAction>>(
    () => new Map(),
  );
  /** Lightweight tick counter so the per-row elapsed chip refreshes
   *  every second WITHOUT re-running the full list memo. Components
   *  that don't need it ignore the value. */
  const [tickMs, setTickMs] = useState(() => Date.now());

  // Reset everything when the workspace changes — mirrors the page-level
  // reset the page previously did inline.
  useEffect(() => {
    setRunningCaseIds(new Set());
    setBatchTotal(null);
    setBatchDone(0);
    setBatchCancelling(false);
    setBatchStartedAtMs(null);
    setBatchFirstCaseMs(null);
    setCasePhases(new Map());
    setRowBusy(new Map());
  }, [workspaceId]);

  // Tick every second while a batch is running so the elapsed chips
  // update. Stops when nothing is in flight to keep React idle.
  useEffect(() => {
    if (batchStartedAtMs === null && casePhases.size === 0) return;
    const id = window.setInterval(() => setTickMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [batchStartedAtMs, casePhases.size]);

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
          if (msg.payload.workspace_id !== workspaceId) return;
          // A new draft appeared — refresh the list to pop it in.
          onRefresh();
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
          onRefresh();
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
          onScheduleRefresh();
        } else if (ev.kind === "case_failed") {
          setBatchDone((d) => d + 1);
          // Drop phase tracking so the failed row doesn't display a
          // stale "active" chip on top of the new error banner.
          setCasePhases((prev) => {
            if (prev.size === 0) return prev;
            return new Map();
          });
          onScheduleRefresh();
        } else if (ev.kind === "case_cancelled") {
          setBatchDone((d) => d + 1);
          setCasePhases((prev) => {
            if (prev.size === 0) return prev;
            return new Map();
          });
          // Re-list so the cancelled row reflects its terminal `failed`
          // status — that's what clears the stuck "Cancelling…" chip
          // (the event carries no case_id, so we lean on refresh's
          // status-based `rowBusy` cleanup).
          onScheduleRefresh();
        } else if (ev.kind === "batch_done") {
          setBatchTotal(null);
          setBatchDone(0);
          setBatchStartedAtMs(null);
          setBatchFirstCaseMs(null);
          setBatchCancelling(false);
          setCasePhases(new Map());
          // Terminal event — refresh now and drop any coalesced refresh.
          onRefreshNow();
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
    // Callbacks are intentionally captured at bind time (once per
    // workspace), preserving the original page-level effect's closure
    // semantics. Do not add them to the deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  /** Cancel the entire batch: skips queued cases AND aborts the in-flight
   *  one (the backend flips every per-case flag). The banner button reads
   *  "Cancelling batch…" until `batch_done` clears `batchCancelling`. */
  const cancelBatch = () => {
    setBatchCancelling(true);
    void ipc.batchCancel().catch(() => setBatchCancelling(false));
  };

  return {
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
  };
}
