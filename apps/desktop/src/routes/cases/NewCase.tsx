// The new-case form: clinical text, question, provider/skill pickers,
// attachments, privacy toggles, data-boundary preview and the
// deliberation launch. Moved verbatim from Cases.tsx.

import { useEffect, useMemo, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { IconLock, IconX } from "@tabler/icons-react";

import { Button } from "../../components/Button";
import { Card, CardBody, CardHeader } from "../../components/Card";
import { Field, Input, Textarea } from "../../components/Field";
import { cn } from "../../lib/cn";
import {
  ipc,
  usableProviders,
  type CaseAttachment,
  type CaseDetail,
  type DataBoundaryMode,
  type DataBoundaryPreview,
  type ProviderInfo,
  type Skill,
  type Workspace,
} from "../../lib/ipc";
import { isReady } from "../../lib/providerStatus";
import { isClinicalEligible, preferredProvider } from "../../lib/providers";
import {
  ModeSelector,
  ProviderOfflineBanner,
} from "./banners";
import { DeliberationOverlay } from "./DeliberationOverlay";
import {
  attachmentBadgeColor,
  attachmentFromPath,
  dedupeAttachments,
  formatBytes,
  SUPPORTED_ATTACHMENT_EXTS,
  type PendingAttachment,
} from "./helpers";
import { ProviderField } from "./ProviderField";

export function NewCase({
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
