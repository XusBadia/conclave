// The case detail view: verdict, audit metadata, attachments section,
// deliberation trace and PDF export. Moved verbatim from Cases.tsx.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconAdjustments, IconFileTypePdf } from "@tabler/icons-react";

import { exportCaseVerdictToPDF } from "../../pdf/exportCaseVerdict";
import { loadPdfExportOptions } from "../../pdf/exportOptions";
import { PdfOptionsSheet } from "../../pdf/PdfOptionsSheet";
import { Button } from "../../components/Button";
import { Card, CardBody, CardHeader } from "../../components/Card";
import { cn } from "../../lib/cn";
import {
  ipc,
  type CaseAttachment,
  type CaseDetail,
  type Workspace,
} from "../../lib/ipc";
import { metaFor } from "../../lib/providers";
import { ShowCaseSkeleton } from "./banners";
import { ConfirmPurgePopover } from "./dialogs";
import {
  DeliberationTraceAccordion,
  FailedCaseErrorBlock,
} from "./DeliberationOverlay";
import { attachmentBadgeColor, formatBytes } from "./helpers";
import { VerdictRenderer } from "./VerdictRenderer";

export function ShowCase({
  workspace,
  detail: initialDetail,
  onBack,
}: {
  workspace: Workspace;
  /** `null` during the optimistic transition while showCase is still
   *  fetching — we render a skeleton placeholder so the user sees an
   *  immediate response instead of a blank page. */
  detail: CaseDetail | null;
  onBack: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [pdfOptionsOpen, setPdfOptionsOpen] = useState(false);
  const [localDetail, setLocalDetail] = useState<CaseDetail | null>(initialDetail);
  const [purgePhiAnchor, setPurgePhiAnchor] = useState<HTMLElement | null>(null);
  const [purgePhiError, setPurgePhiError] = useState<string | null>(null);
  const [purgeAttachmentsAnchor, setPurgeAttachmentsAnchor] =
    useState<HTMLElement | null>(null);
  const [purgeAttachmentsError, setPurgeAttachmentsError] = useState<string | null>(null);

  useEffect(() => {
    setLocalDetail(initialDetail);
  }, [initialDetail]);

  const feedback = async (kind: "accept" | "modify" | "reject") => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.submitFeedback({
        workspace_id: workspace.id,
        case_id: current.case.id,
        kind,
      });
      alert(t("cases.feedback_recorded", { kind }));
      const refreshed = await ipc.showCase(workspace.id, current.case.id);
      if (refreshed) setLocalDetail(refreshed);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const purgePhi = async () => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setPurgePhiError(null);
    try {
      const purged = await ipc.purgeCasePhi(workspace.id, current.case.id);
      setLocalDetail({ ...current, case: purged });
      setPurgePhiAnchor(null);
    } catch (e) {
      setPurgePhiError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const purgeAttachments = async () => {
    const current = localDetail;
    if (!current) return;
    setBusy(true);
    setPurgeAttachmentsError(null);
    try {
      await ipc.purgeCaseAttachments(workspace.id, current.case.id);
      const refreshed = await ipc.showCase(workspace.id, current.case.id);
      if (refreshed) setLocalDetail(refreshed);
      setPurgeAttachmentsAnchor(null);
    } catch (e) {
      setPurgeAttachmentsError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const exportPdf = async () => {
    const current = localDetail;
    if (!current?.verdict) return;
    setExporting(true);
    setError(null);
    try {
      await exportCaseVerdictToPDF(current, t, i18n.language, loadPdfExportOptions());
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  };

  if (!localDetail) {
    return <ShowCaseSkeleton onBack={onBack} />;
  }

  const detail = localDetail;

  return (
    <div className="mx-auto w-full max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onBack}>
          {t("cases.back")}
        </Button>
        {detail.verdict && (
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setPdfOptionsOpen(true)}
              disabled={busy}
              aria-label={t("cases.export_options")}
              title={t("cases.export_options")}
            >
              <IconAdjustments size={15} stroke={1.6} />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={exportPdf}
              loading={exporting}
              disabled={busy}
            >
              <IconFileTypePdf size={14} className="mr-1" />
              {t("cases.export_pdf")}
            </Button>
            {detail.case.raw_text_retention !== "discarded" && (
              <Button
                size="sm"
                variant="ghost"
                onClick={(e) => {
                  setPurgePhiError(null);
                  setPurgePhiAnchor(e.currentTarget);
                }}
                loading={busy && purgePhiAnchor !== null}
              >
                {t("cases.purge_phi")}
              </Button>
            )}
            {detail.attachments.some((a) => a.stored_path) && (
              <Button
                size="sm"
                variant="ghost"
                onClick={(e) => {
                  setPurgeAttachmentsError(null);
                  setPurgeAttachmentsAnchor(e.currentTarget);
                }}
                loading={busy && purgeAttachmentsAnchor !== null}
              >
                {t("cases.purge_attachments")}
              </Button>
            )}
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

      <ConfirmPurgePopover
        open={purgePhiAnchor !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPurgePhiAnchor(null);
            setPurgePhiError(null);
          }
        }}
        anchor={purgePhiAnchor}
        busy={busy && purgePhiAnchor !== null}
        error={purgePhiError}
        title={t("cases.purge_phi_confirm_title")}
        lines={[
          t("cases.purge_phi_removed"),
          t("cases.purge_phi_kept"),
          t("cases.purge_phi_irreversible"),
        ]}
        confirmLabel={t("cases.purge_phi_confirm_action")}
        onConfirm={purgePhi}
      />

      <ConfirmPurgePopover
        open={purgeAttachmentsAnchor !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPurgeAttachmentsAnchor(null);
            setPurgeAttachmentsError(null);
          }
        }}
        anchor={purgeAttachmentsAnchor}
        busy={busy && purgeAttachmentsAnchor !== null}
        error={purgeAttachmentsError}
        title={t("cases.purge_attachments_confirm_title")}
        lines={[
          t("cases.purge_attachments_removed"),
          t("cases.purge_attachments_kept"),
          t("cases.purge_attachments_original_safe"),
        ]}
        confirmLabel={t("cases.purge_attachments_confirm_action")}
        onConfirm={purgeAttachments}
      />

      <PdfOptionsSheet open={pdfOptionsOpen} onOpenChange={setPdfOptionsOpen} />

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
                {metaFor(detail.verdict_record.provider_id).name} ·{" "}
                {detail.verdict_record.model} ·{" "}
                {detail.verdict_record.latency_ms}ms
              </span>
            )
          }
        />
        <CardBody className="space-y-6 prose-conclave">
          {detail.case.status === "failed" && detail.case.latest_error && (
            <FailedCaseErrorBlock error={detail.case.latest_error} />
          )}
          {detail.review && (
            <div className="rounded-md border border-ok/30 bg-ok/5 px-3 py-2 text-[12.5px] text-ok">
              {t("cases.review_finalized", {
                decision: detail.review.decision,
                date: new Date(detail.review.reviewed_at).toLocaleString(),
              })}
              {detail.review.diff_summary && (
                <div className="mt-1 text-ink-dim">{detail.review.diff_summary}</div>
              )}
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

      {detail.audit && (
        <Card>
          <CardHeader
            title={t("cases.audit_title")}
            subtitle={`${detail.audit.provider_id} · ${detail.audit.model}`}
          />
          <CardBody>
            <div className="grid gap-2 text-[12px] text-ink-dim md:grid-cols-2">
              <AuditRow label={t("cases.audit_mode")} value={detail.audit.data_boundary_mode} />
              <AuditRow label={t("cases.audit_payload")} value={detail.audit.payload_mode} />
              <AuditRow label={t("cases.audit_prompt_hash")} value={detail.audit.prompt_sha256} mono />
              <AuditRow label={t("cases.audit_output_hash")} value={detail.audit.output_sha256} mono />
              <AuditRow label={t("cases.audit_raw_retention")} value={detail.audit.raw_text_retention} />
              <AuditRow
                label={t("cases.audit_refs")}
                value={[
                  ...detail.audit.evidence_refs,
                  ...detail.audit.attachment_refs,
                  ...detail.audit.past_cases_refs,
                  ...detail.audit.online_evidence_refs,
                ].join(", ") || "—"}
              />
            </div>
          </CardBody>
        </Card>
      )}

      <Card>
        <CardHeader
          title={t("cases.attachments_section_title")}
          subtitle={t("cases.attachments_section_subtitle")}
        />
        <CardBody>
          <p className="mb-3 text-[12px] leading-relaxed text-ink-faint">
            {t("cases.attachments_storage_hint")}
          </p>
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

function AuditRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md border border-border-subtle bg-bg px-2.5 py-2">
      <div className="text-[10.5px] uppercase tracking-wide text-ink-faint">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 min-w-0 truncate text-ink",
          mono && "font-mono text-[11px]",
        )}
        title={value}
      >
        {value}
      </div>
    </div>
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
              {a.stored_path ? (
                <span className="shrink-0 text-[11px] text-ink-faint">
                  {formatBytes(a.byte_size)}
                </span>
              ) : (
                <span
                  className="shrink-0 rounded bg-surface px-1.5 py-0.5 text-[10px] font-medium text-ink-subtle"
                  title={t("cases.purge_attachments_kept")}
                >
                  {t("cases.attachment_purged_badge")}
                </span>
              )}
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
