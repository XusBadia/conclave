// Characterization tests for the batch/live-progress state machine,
// written as it was extracted from Cases.tsx. They pin the event
// semantics the backend relies on: queued→started→completed/failed/
// cancelled ordering, batch_done reset, deliberation phase lifecycle,
// workspace filtering and listener cleanup.
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (msg: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();
const unlistenSpies = new Map<string, ReturnType<typeof vi.fn>>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: Handler) => {
    handlers.set(name, cb);
    const un = vi.fn(() => handlers.delete(name));
    unlistenSpies.set(name, un);
    return Promise.resolve(un);
  }),
}));

const batchCancel = vi.fn(() => Promise.resolve());
vi.mock("../../lib/ipc", () => ({
  ipc: { batchCancel: () => batchCancel() },
}));

import { useBatchProgress } from "./useBatchProgress";

function setup(workspaceId = "ws-1") {
  const onRefresh = vi.fn();
  const onScheduleRefresh = vi.fn();
  const onRefreshNow = vi.fn();
  const utils = renderHook(
    (props: { workspaceId: string }) =>
      useBatchProgress({
        workspaceId: props.workspaceId,
        onRefresh,
        onScheduleRefresh,
        onRefreshNow,
      }),
    { initialProps: { workspaceId } },
  );
  return { ...utils, onRefresh, onScheduleRefresh, onRefreshNow };
}

const emit = (event: string, payload: unknown) =>
  act(() => {
    handlers.get(event)?.({ payload });
  });

beforeEach(() => {
  handlers.clear();
  unlistenSpies.clear();
  batchCancel.mockClear();
});

describe("useBatchProgress", () => {
  it("counts queued cases and stamps the batch start once", async () => {
    const { result } = setup();
    await act(async () => {});
    await emit("batch:progress", { kind: "case_queued", index: 0, patient_label: "A" });
    await emit("batch:progress", { kind: "case_queued", index: 1, patient_label: "B" });
    expect(result.current.batchTotal).toBe(2);
    expect(result.current.batchStartedAtMs).not.toBeNull();
  });

  it("case_completed settles the row and schedules a coalesced refresh", async () => {
    const { result, onScheduleRefresh } = setup();
    await act(async () => {});
    await emit("deliberation:progress", {
      kind: "phase_started",
      case_id: "c1",
      phase: "briefing",
    });
    expect(result.current.casePhases.get("c1")?.status).toBe("active");

    await emit("batch:progress", { kind: "case_completed", case_id: "c1", index: 0 });
    expect(result.current.batchDone).toBe(1);
    expect(result.current.casePhases.has("c1")).toBe(false);
    expect(result.current.runningCaseIds.has("c1")).toBe(false);
    expect(onScheduleRefresh).toHaveBeenCalled();
  });

  it("case_failed and case_cancelled count as done and wipe phase chips", async () => {
    const { result, onScheduleRefresh } = setup();
    await act(async () => {});
    await emit("deliberation:progress", {
      kind: "phase_started",
      case_id: "c1",
      phase: "drafting",
    });
    await emit("batch:progress", { kind: "case_failed", index: 0, patient_label: "A", error: "boom" });
    expect(result.current.batchDone).toBe(1);
    expect(result.current.casePhases.size).toBe(0);
    await emit("batch:progress", { kind: "case_cancelled", index: 1, patient_label: "B" });
    expect(result.current.batchDone).toBe(2);
    expect(onScheduleRefresh).toHaveBeenCalledTimes(2);
  });

  it("batch_done resets the whole machine and refreshes immediately", async () => {
    const { result, onRefreshNow } = setup();
    await act(async () => {});
    await emit("batch:progress", { kind: "case_queued", index: 0, patient_label: "A" });
    await emit("batch:progress", { kind: "case_completed", case_id: "c1", index: 0 });
    await emit("batch:progress", {
      kind: "batch_done",
      completed: 1,
      failed: 0,
      cancelled: 0,
    });
    expect(result.current.batchTotal).toBeNull();
    expect(result.current.batchDone).toBe(0);
    expect(result.current.batchStartedAtMs).toBeNull();
    expect(result.current.batchCancelling).toBe(false);
    expect(result.current.casePhases.size).toBe(0);
    expect(onRefreshNow).toHaveBeenCalledTimes(1);
  });

  it("tracks the deliberation phase lifecycle per case id", async () => {
    const { result } = setup();
    await act(async () => {});
    await emit("deliberation:progress", {
      kind: "phase_started",
      case_id: "c9",
      phase: "redteam",
    });
    const started = result.current.casePhases.get("c9");
    expect(started).toMatchObject({ phase: "redteam", status: "active" });

    await emit("deliberation:progress", {
      kind: "phase_completed",
      case_id: "c9",
      phase: "redteam",
      output: "…",
      elapsed_ms: 10,
    });
    const completed = result.current.casePhases.get("c9");
    expect(completed?.status).toBe("done");
    // startedAtMs is preserved from the active entry.
    expect(completed?.startedAtMs).toBe(started?.startedAtMs);

    await emit("deliberation:progress", { kind: "done", case_id: "c9" });
    expect(result.current.casePhases.has("c9")).toBe(false);
  });

  it("only refreshes for case:drafted events from the active workspace", async () => {
    const { onRefresh } = setup("ws-1");
    await act(async () => {});
    await emit("case:drafted", { workspace_id: "ws-other", case_id: "x" });
    expect(onRefresh).not.toHaveBeenCalled();
    await emit("case:drafted", { workspace_id: "ws-1", case_id: "x" });
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("cancelBatch flips the cancelling flag and calls the backend once", async () => {
    const { result } = setup();
    await act(async () => {});
    act(() => {
      result.current.cancelBatch();
    });
    expect(result.current.batchCancelling).toBe(true);
    expect(batchCancel).toHaveBeenCalledTimes(1);
  });

  it("unsubscribes every listener on unmount", async () => {
    const { unmount } = setup();
    await act(async () => {});
    expect(handlers.size).toBe(3);
    unmount();
    for (const un of unlistenSpies.values()) {
      expect(un).toHaveBeenCalled();
    }
  });
});
