// Structured rendering of a Verdict: summary, key data, the committed
// recommendation, certainty, red flags, follow-up triggers and evidence.
// The page leads with the recommendation; supporting detail (clinical data,
// red flags, triggers, applied evidence) is collapsed by default. Shared by
// the case detail view and the deliberation overlay (compact mode skips the
// disclaimer block the parent already shows).

import { useTranslation } from "react-i18next";
import { IconAlertTriangle, IconDatabase } from "@tabler/icons-react";

import type { Verdict } from "../../lib/ipc";
import { CopyButton } from "./banners";
import { CollapsibleSection, InfoTip } from "./CollapsibleSection";

export function VerdictRenderer({
  verdict,
  compact = false,
}: {
  verdict: Verdict;
  /** When `true`, skip the final disclaimer block. Used when the renderer
   *  is embedded inside a deliberation phase view where the same
   *  disclaimer is already shown on the parent case page. */
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const certaintyColor =
    verdict.certainty_level === "high"
      ? "text-ok"
      : verdict.certainty_level === "medium"
        ? "text-accent"
        : "text-warn";

  const primaryRecText = `${verdict.primary_recommendation.action}\n\n${verdict.primary_recommendation.rationale}`;

  return (
    <div className="space-y-6">
      <section>
        <SectionRow
          title={t("cases.verdict.case_summary")}
          copyText={verdict.case_summary}
        />
        <p>{verdict.case_summary}</p>
      </section>

      {verdict.key_clinical_data.length > 0 && (
        <CollapsibleSection
          title={t("cases.verdict.key_clinical_data")}
          count={verdict.key_clinical_data.length}
          helpText={t("cases.verdict.help.key_clinical_data")}
          copyText={verdict.key_clinical_data
            .map((kv) => `${kv.label}: ${kv.value}`)
            .join("\n")}
        >
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
        </CollapsibleSection>
      )}

      <section>
        <SectionRow
          title={t("cases.verdict.primary_recommendation")}
          helpText={t("cases.verdict.help.primary_recommendation")}
          copyText={primaryRecText}
        />
        <div className="border border-border-strong bg-surface px-4 py-3">
          <div className="text-[14px] font-semibold text-ink">
            {verdict.primary_recommendation.action}
          </div>
          <div className="mt-1 text-[13px] text-ink-dim">
            {verdict.primary_recommendation.rationale}
          </div>
        </div>
      </section>

      <section>
        <SectionRow
          title={t("cases.verdict.certainty")}
          helpText={t("cases.verdict.help.certainty")}
          copyText={`${verdict.certainty_level.toUpperCase()} — ${verdict.certainty_justification}`}
        />
        <div className={`text-[14px] font-semibold ${certaintyColor}`}>
          {verdict.certainty_level.toUpperCase()}
        </div>
        <p className="mt-1">{verdict.certainty_justification}</p>
        {verdict.data_completeness && (
          <div
            className="mt-2 inline-flex items-center gap-1.5 rounded-md border border-border-subtle bg-bg px-2 py-1 text-[11px] uppercase tracking-wide text-ink-faint"
            title={t("cases.verdict.help.data_completeness")}
          >
            <IconDatabase size={12} stroke={1.7} aria-hidden />
            {t("cases.verdict.data_completeness")}:{" "}
            {t(
              `cases.verdict.data_completeness_value.${verdict.data_completeness}`,
            )}
          </div>
        )}
      </section>

      {verdict.red_flags.length > 0 && (
        <CollapsibleSection
          title={t("cases.verdict.red_flags")}
          count={verdict.red_flags.length}
          tone="warn"
          helpText={t("cases.verdict.help.red_flags")}
          copyText={verdict.red_flags.map((rf) => `• ${rf}`).join("\n")}
        >
          <ul className="space-y-1.5">
            {verdict.red_flags.map((rf, i) => (
              <li
                key={i}
                className="flex items-start gap-2 rounded-md border border-warn/40 bg-warn/5 px-3 py-2 text-[13px] text-ink-dim"
              >
                <IconAlertTriangle
                  size={14}
                  stroke={1.7}
                  aria-hidden
                  className="mt-0.5 shrink-0 text-warn"
                />
                <span>{rf}</span>
              </li>
            ))}
          </ul>
        </CollapsibleSection>
      )}

      {verdict.follow_up_triggers.length > 0 && (
        <CollapsibleSection
          title={t("cases.verdict.follow_up_triggers")}
          count={verdict.follow_up_triggers.length}
          helpText={t("cases.verdict.help.follow_up_triggers")}
          copyText={verdict.follow_up_triggers
            .map((tr) => `• ${tr}`)
            .join("\n")}
        >
          <ul className="list-inside list-disc space-y-1 text-[13px] text-ink-dim">
            {verdict.follow_up_triggers.map((tr, i) => (
              <li key={i}>{tr}</li>
            ))}
          </ul>
        </CollapsibleSection>
      )}

      {verdict.applied_evidence.length > 0 && (
        <CollapsibleSection
          title={t("cases.verdict.applied_evidence")}
          count={verdict.applied_evidence.length}
          helpText={t("cases.verdict.help.applied_evidence")}
          copyText={verdict.applied_evidence
            .map((ev) => `[${ev.ref}] ${ev.claim}`)
            .join("\n")}
        >
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
        </CollapsibleSection>
      )}

      {!compact && (
        <section>
          <SectionTitle>{t("cases.verdict.disclaimer")}</SectionTitle>
          <p className="text-[12px] leading-relaxed text-ink-subtle">
            {t("cases.verdict.disclaimer_body")}
          </p>
        </section>
      )}
    </div>
  );
}

export function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-1.5 text-[11px] uppercase tracking-[0.08em] text-ink-faint">
      {children}
    </h4>
  );
}

/** Section title + an optional info tooltip + a compact copy-to-clipboard
 *  affordance. Clinicians routinely paste recommendations into EHR notes, so
 *  every major verdict block gets one. */
export function SectionRow({
  title,
  copyText,
  helpText,
}: {
  title: string;
  copyText: string;
  helpText?: string;
}) {
  return (
    <div className="mb-1.5 flex items-center justify-between gap-2">
      <div className="flex items-center gap-1.5">
        <SectionTitle>{title}</SectionTitle>
        {helpText && <InfoTip text={helpText} />}
      </div>
      <CopyButton text={copyText} />
    </div>
  );
}
