// The deliberation surface: the in-flight committee overlay (with its
// own `deliberation:progress` listener), per-phase rows, the post-hoc
// trace accordion shown in the case detail, and the failed-case error
// block. State is local to these components — the page only toggles
// visibility.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconClipboardCheck,
  IconPencil,
  IconRefresh,
  IconShield,
  IconStethoscope,
  IconX,
} from "@tabler/icons-react";

import { Button } from "../../components/Button";
import { Card, CardBody, CardHeader } from "../../components/Card";
import { ProviderStatusPill } from "../../components/ProviderStatusPill";
import { cn } from "../../lib/cn";
import {
  ipc,
  type DeliberationEvent,
  type DeliberationPhase,
  type DeliberationTrace,
  type ProviderInfo,
} from "../../lib/ipc";
import { metaFor } from "../../lib/providers";
import { formatElapsed } from "./helpers";
import { parseFailedPhase, PHASE_ORDER, tryParseVerdict } from "./verdictParsing";
import { VerdictRenderer } from "./VerdictRenderer";

export function FailedCaseErrorBlock({ error }: { error: string }) {
  const { t } = useTranslation();
  const phase = parseFailedPhase(error);
  const title = phase
    ? t("cases.failed_phase_title", {
        phase: t(`cases.phases.${phase}`),
      })
    : t("cases.failed_title");
  return (
    <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
      <div className="font-medium mb-1">{title}</div>
      <details className="text-[12px]">
        <summary className="cursor-pointer select-none text-danger/80 hover:text-danger">
          {t("cases.failed_show_details")}
        </summary>
        <div className="mt-1 whitespace-pre-wrap break-words font-mono">
          {error}
        </div>
      </details>
    </div>
  );
}

type PhaseState = {
  status: "pending" | "active" | "retrying" | "done" | "failed";
  output?: string;
  error?: string;
  /** Wall-clock duration of this phase as reported by the backend.
   *  Present only on `done` / `failed` — pending/active rows omit it. */
  elapsedMs?: number;
  /** Upcoming attempt number when status === "retrying" (e.g. 2 after
   *  the first transient failure). Used by the badge to render
   *  "Retrying (2/2)". */
  retryAttempt?: number;
  /** Short human-readable cause for the retry (e.g. "network: …").
   *  Shown next to the badge so the user knows why we're retrying. */
  retryReason?: string;
};

function PhaseIcon({
  phase,
  className,
}: {
  phase: DeliberationPhase;
  className?: string;
}) {
  const props = {
    size: 16,
    stroke: 1.6,
    "aria-hidden": true,
    className,
  } as const;
  switch (phase) {
    case "briefing":
      return <IconStethoscope {...props} />;
    case "drafting":
      return <IconPencil {...props} />;
    case "redteam":
      return <IconShield {...props} />;
    case "finalize":
      return <IconClipboardCheck {...props} />;
  }
}

/** Render one phase's `output`. `drafting` and `finalize` produce JSON;
 *  parse them and render through `VerdictRenderer` so the user sees the
 *  structured verdict, not raw `{...}` text. `briefing` and `redteam` are
 *  markdown — pass straight to `ReactMarkdown`. If a JSON phase fails to
 *  parse (rare retry-then-fail path), show the raw payload in a `<pre>`
 *  block so the underlying error is still inspectable. */
function PhaseOutput({
  phase,
  output,
}: {
  phase: DeliberationPhase;
  output: string;
}) {
  if (phase === "drafting" || phase === "finalize") {
    const parsed = tryParseVerdict(output);
    if (parsed) {
      return <VerdictRenderer verdict={parsed} compact />;
    }
    return (
      <pre className="max-h-[400px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg p-3 font-mono text-[11.5px] leading-relaxed text-ink-dim">
        {output}
      </pre>
    );
  }
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]}>{output}</ReactMarkdown>
  );
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
export function DeliberationOverlay({
  provider,
  onDismiss,
}: {
  /** Provider executing the committee. Rendered in the overlay
   *  header so an auth/network failure mid-run lands next to the
   *  pill that explains what happened. */
  provider: ProviderInfo | null;
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
            } else if (ev.kind === "phase_retrying") {
              // Preserve any partial output the active attempt produced
              // and surface the upcoming attempt number + reason. The
              // backend will follow this with another implicit "active"
              // run; the next phase_completed/phase_failed resets the row.
              next[ev.phase] = {
                ...prev[ev.phase],
                status: "retrying",
                retryAttempt: ev.attempt,
                retryReason: ev.reason,
              };
              setExpanded(ev.phase);
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
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
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
            </div>
            {/* Provider info pinned to the right of the modal header.
                If the LLM fails mid-run, the pill flips to amber and
                the user sees the cause inline with the failing phase
                row — no more orphaned red boxes. */}
            {provider && (
              <div className="flex shrink-0 items-center gap-2 rounded-md border border-border-subtle bg-bg px-2 py-1">
                <span
                  aria-hidden
                  className="grid h-5 w-5 place-content-center rounded bg-slate-400/10 text-[10px] font-semibold text-ink-dim ring-1 ring-border-subtle"
                >
                  {metaFor(provider.id).monogram}
                </span>
                <span className="truncate text-[11.5px] font-medium text-ink-dim">
                  {metaFor(provider.id).name}
                </span>
                <ProviderStatusPill status={provider.status} size="sm" />
              </div>
            )}
          </div>
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
      case "retrying":
        return (
          <span className="flex items-center gap-1.5 rounded bg-amber-400/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-amber-200">
            <IconRefresh size={12} stroke={2} className="animate-spin" aria-hidden />
            {t("cases.phase_status_retrying", {
              attempt: state.retryAttempt ?? 2,
            })}
          </span>
        );
      case "done":
        return (
          <span className="flex items-center gap-1.5 rounded bg-ok/15 px-2 py-0.5 text-[10.5px] font-medium uppercase tracking-wide text-ok">
            <span className="flex items-center gap-1">
              <IconCheck size={12} stroke={2} aria-hidden />
              {t("cases.phase_status_done")}
            </span>
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
            <span className="flex items-center gap-1">
              <IconX size={12} stroke={2} aria-hidden />
              {t("cases.phase_status_failed")}
            </span>
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
        state.status === "active" && "border-accent/60 bg-accent/5",
        state.status === "retrying" && "border-amber-400/60 bg-amber-400/5",
        state.status !== "active" &&
          state.status !== "retrying" &&
          "border-border-subtle bg-bg",
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
        <PhaseIcon phase={phase} className="shrink-0 text-ink-subtle" />
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
      {expanded && (state.output || state.error || state.retryReason) && (
        <div className="border-t border-border-subtle px-3 py-3 space-y-2">
          {state.status === "retrying" && state.retryReason && (
            <div className="rounded-md border border-amber-400/40 bg-amber-400/10 px-3 py-2 text-[12px] text-amber-200">
              <span className="font-medium">
                {t("cases.phase_retry_caption", {
                  attempt: state.retryAttempt ?? 2,
                })}
              </span>
              <span className="ml-2 font-mono text-[11.5px] opacity-90">
                {state.retryReason}
              </span>
            </div>
          )}
          {state.error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[12px] text-danger">
              {state.error}
            </div>
          )}
          {state.output && (
            <div className="prose-conclave max-h-[360px] overflow-auto text-[12.5px]">
              <PhaseOutput phase={phase} output={state.output} />
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
export function DeliberationTraceAccordion({
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
        subtitle={
          totalTokens > 0
            ? t("cases.deliberation_trace_subtitle", {
                tokens: totalTokens,
                duration: seconds,
              })
            : t("cases.deliberation_trace_subtitle_notokens", {
                duration: seconds,
              })
        }
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
                <PhaseIcon phase={phase} className="shrink-0 text-ink-subtle" />
                <span className="flex-1 text-[13px] font-medium text-ink">
                  {t(`cases.deliberation_phase.${phase}_title`)}
                </span>
                {expanded === phase ? (
                  <IconChevronDown
                    size={14}
                    stroke={1.6}
                    aria-hidden
                    className="text-ink-faint"
                  />
                ) : (
                  <IconChevronRight
                    size={14}
                    stroke={1.6}
                    aria-hidden
                    className="text-ink-faint"
                  />
                )}
              </button>
              {expanded === phase && output && (
                <div className="border-t border-border-subtle px-3 py-3">
                  <div className="prose-conclave max-h-[400px] overflow-auto text-[12.5px]">
                    <PhaseOutput phase={phase} output={output} />
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
