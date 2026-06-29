import {
  Document,
  Page,
  StyleSheet,
  Text,
  View,
  Font,
} from "@react-pdf/renderer";
import type { ReactNode } from "react";
import type { TFunction } from "i18next";
import type {
  CaseAttachment,
  CaseDetail,
  CaseRecord,
  ReviewMetadataRecord,
  Verdict,
  VerdictRecord,
} from "../lib/ipc";
import {
  DEFAULT_PDF_EXPORT_OPTIONS,
  type PdfExportOptions,
} from "./exportOptions";

/* ------------------------------------------------------------------ */
/* Fonts                                                              */
/* ------------------------------------------------------------------ */

/* @react-pdf ships with Helvetica / Helvetica-Bold / Helvetica-Oblique /
 * Courier baked in as the standard PDF base 14. They render Latin-1 cleanly
 * (ñ á é í ó ú ¿ ¡) and require no font files on disk, which keeps the bundle
 * small and avoids the Tauri asset-protocol "font not found" footgun. */
Font.registerHyphenationCallback((word) => [word]);

/* ------------------------------------------------------------------ */
/* Design tokens                                                      */
/* ------------------------------------------------------------------ */

const color = {
  paper: "#FAFAF8",
  ink: "#121214",
  inkDim: "#3C3C40",
  inkSubtle: "#646469",
  inkFaint: "#96969B",
  hairline: "#E6E6E2",
  hairlineStrong: "#CFCFC9",
  surfaceSoft: "#F4F4F0",
  accent: "#0E7490",
  accentSoft: "#E0F2FE",
  ok: "#15803D",
  okSoft: "#DCFCE7",
  warn: "#A16207",
  warnSoft: "#FEF3C7",
  danger: "#B91C1C",
  dangerSoft: "#FEE2E2",
  dangerWash: "#FEF2F2",
} as const;

const statusPalette: Record<CaseRecord["status"], { text: string; bg: string }> = {
  draft: { text: color.inkSubtle, bg: color.surfaceSoft },
  review_ready: { text: color.warn, bg: color.warnSoft },
  finalized: { text: color.ok, bg: color.okSoft },
  finalized_legacy: { text: color.inkSubtle, bg: color.surfaceSoft },
  failed: { text: color.danger, bg: color.dangerSoft },
};

const certaintyPalette: Record<Verdict["certainty_level"], { text: string; bg: string }> = {
  high: { text: color.ok, bg: color.okSoft },
  medium: { text: color.warn, bg: color.warnSoft },
  low: { text: color.danger, bg: color.dangerSoft },
};

const reviewPalette: Record<ReviewMetadataRecord["decision"], { text: string; bg: string }> = {
  accept: { text: color.ok, bg: color.okSoft },
  modify: { text: color.warn, bg: color.warnSoft },
  reject: { text: color.danger, bg: color.dangerSoft },
};

const styles = StyleSheet.create({
  page: {
    backgroundColor: color.paper,
    paddingTop: 48,
    paddingBottom: 56,
    paddingHorizontal: 52,
    fontFamily: "Helvetica",
    fontSize: 9.5,
    color: color.inkDim,
    lineHeight: 1.35,
  },

  /* Header — vertical stack: wordmark, title, full-width label, meta strip.
   * The old two-column layout (fixed 200pt right column) let long patient
   * labels overprint the metadata; stacking gives the label the full width. */
  header: { marginBottom: 18 },
  wordmark: {
    fontFamily: "Helvetica-Bold",
    fontSize: 10,
    color: color.ink,
    letterSpacing: 0.8,
    textTransform: "uppercase",
    marginBottom: 10,
  },
  h1: {
    fontFamily: "Helvetica-Bold",
    fontSize: 15,
    color: color.ink,
    lineHeight: 1.25,
  },
  patientLabel: {
    fontFamily: "Helvetica-Bold",
    fontSize: 10,
    color: color.inkDim,
    marginTop: 6,
  },
  /* Metadata strip: a wrapping row of LABEL·value items below the title. */
  metaStrip: { flexDirection: "row", flexWrap: "wrap", marginTop: 10 },
  metaItem: {
    flexDirection: "row",
    alignItems: "center",
    marginRight: 16,
    marginBottom: 2,
  },
  metaItemLabel: {
    fontFamily: "Helvetica",
    fontSize: 8,
    color: color.inkSubtle,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    marginRight: 5,
  },
  metaItemValue: { fontFamily: "Helvetica", fontSize: 9, color: color.ink },
  metaItemValueMono: { fontFamily: "Courier", fontSize: 8.5, color: color.ink },
  headerRule: {
    marginTop: 14,
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairlineStrong,
  },

  /* Section primitives */
  section: { marginBottom: 14 },
  sectionHeader: { marginBottom: 6 },
  eyebrow: {
    fontFamily: "Helvetica-Bold",
    fontSize: 9,
    color: color.ink,
    textTransform: "uppercase",
    letterSpacing: 0.6,
  },
  eyebrowAccent: { color: color.accent },
  eyebrowDanger: { color: color.danger },
  body: {
    fontFamily: "Helvetica",
    fontSize: 9.5,
    color: color.inkDim,
    lineHeight: 1.35,
  },
  bodyStrong: {
    fontFamily: "Helvetica-Bold",
    fontSize: 9.5,
    color: color.ink,
  },
  paragraphSpacing: { marginBottom: 6 },

  /* Clinical data table */
  tableRow: {
    flexDirection: "row",
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairline,
    paddingVertical: 5,
    paddingHorizontal: 4,
  },
  tableRowZebra: { backgroundColor: color.surfaceSoft },
  tableLabel: {
    width: "32%",
    fontFamily: "Helvetica-Bold",
    fontSize: 9.5,
    color: color.ink,
    paddingRight: 10,
  },
  tableValue: {
    width: "68%",
    fontFamily: "Helvetica",
    fontSize: 9.5,
    color: color.inkDim,
  },

  /* Primary recommendation */
  primaryBox: {
    backgroundColor: color.accentSoft,
    borderLeftWidth: 3,
    borderLeftColor: color.accent,
    paddingVertical: 14,
    paddingHorizontal: 16,
  },
  primaryAction: {
    fontFamily: "Helvetica-Bold",
    fontSize: 11,
    color: color.ink,
    marginTop: 6,
    marginBottom: 8,
    lineHeight: 1.35,
  },

  /* Certainty */
  certaintyRow: {
    flexDirection: "row",
    alignItems: "center",
    marginBottom: 8,
  },
  certaintyPill: {
    paddingVertical: 3,
    paddingHorizontal: 8,
    borderRadius: 4,
    fontFamily: "Helvetica-Bold",
    fontSize: 9,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    marginRight: 10,
  },

  /* Red flags */
  redFlagsBox: {
    borderLeftWidth: 2,
    borderLeftColor: color.danger,
    backgroundColor: color.dangerWash,
    paddingVertical: 12,
    paddingHorizontal: 14,
  },
  bulletRow: { flexDirection: "row", marginBottom: 4 },
  bulletMark: { width: 12, color: color.danger, fontSize: 10 },
  bulletMarkSubtle: { width: 12, color: color.inkSubtle, fontSize: 10 },

  /* Evidence */
  evidenceRow: {
    flexDirection: "row",
    paddingVertical: 6,
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairline,
  },
  evidenceRef: {
    width: 50,
    fontFamily: "Courier",
    fontSize: 9,
    color: color.accent,
  },
  evidenceClaim: { flex: 1, fontSize: 9.5, color: color.inkDim },

  /* Header note (optional letterhead line) */
  headerNote: {
    fontFamily: "Helvetica-Oblique",
    fontSize: 9.5,
    color: color.inkSubtle,
    marginTop: -8,
    marginBottom: 18,
  },

  /* Source documents */
  sourceRow: {
    flexDirection: "row",
    paddingVertical: 5,
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairline,
  },
  sourceRef: {
    width: 36,
    fontFamily: "Courier",
    fontSize: 9,
    color: color.accent,
  },
  sourceName: { flex: 1, fontSize: 9.5, color: color.inkDim },
  sourceMeta: { fontFamily: "Helvetica", fontSize: 8.5, color: color.inkFaint },

  /* Generation details */
  genRow: { flexDirection: "row", marginBottom: 4 },
  genLabel: {
    width: 120,
    fontFamily: "Helvetica",
    fontSize: 8.5,
    color: color.inkSubtle,
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  genValue: { flex: 1, fontFamily: "Helvetica", fontSize: 9.5, color: color.ink },

  /* Review */
  reviewBox: {
    backgroundColor: color.surfaceSoft,
    padding: 12,
  },
  reviewMetaLine: {
    fontFamily: "Helvetica",
    fontSize: 9.5,
    color: color.inkSubtle,
    marginTop: 2,
  },
  reviewNote: {
    fontFamily: "Helvetica-Oblique",
    fontSize: 9.5,
    color: color.inkDim,
    marginTop: 6,
  },

  /* Disclaimer */
  disclaimerBox: {
    backgroundColor: color.surfaceSoft,
    padding: 12,
  },
  disclaimerText: {
    fontFamily: "Helvetica",
    fontSize: 9,
    color: color.inkSubtle,
    lineHeight: 1.45,
    textAlign: "justify",
  },

  /* Footer — text styles live on the <Text> children (not the View) so the
   * ink reliably paints; the View is just the fixed, absolutely-positioned
   * bar. Color darkened from inkFaint so page numbers are actually legible. */
  footer: {
    position: "absolute",
    bottom: 24,
    left: 52,
    right: 52,
    borderTopWidth: 0.5,
    borderTopColor: color.hairline,
    paddingTop: 8,
    flexDirection: "row",
    justifyContent: "space-between",
  },
  footerText: {
    fontFamily: "Helvetica",
    fontSize: 8.5,
    color: color.inkSubtle,
  },
  footerId: {
    fontFamily: "Courier",
    fontSize: 8.5,
    color: color.inkSubtle,
  },
  footerWordmark: {
    fontFamily: "Helvetica-Bold",
    fontSize: 8.5,
    color: color.inkSubtle,
    letterSpacing: 0.8,
    textTransform: "uppercase",
  },
});

/* ------------------------------------------------------------------ */
/* Small helpers                                                      */
/* ------------------------------------------------------------------ */

function formatDate(rfc3339: string, locale: string): string {
  const d = new Date(rfc3339);
  if (Number.isNaN(d.getTime())) return rfc3339;
  return d.toLocaleDateString(locale, {
    year: "numeric",
    month: "short",
    day: "2-digit",
  });
}

function formatDateTime(rfc3339: string, locale: string): string {
  const d = new Date(rfc3339);
  if (Number.isNaN(d.getTime())) return rfc3339;
  return d.toLocaleString(locale, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function statusLabel(t: TFunction, status: CaseRecord["status"]): string {
  const key = `cases.pdf.status.${status}`;
  const fallback = status.replace(/_/g, " ");
  const val = t(key, { defaultValue: fallback });
  return typeof val === "string" ? val : fallback;
}

function nonEmptyText(value: string | null | undefined): string | null {
  if (value === null || value === undefined) return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function splitParagraphs(text: string): string[] {
  return text
    .split(/\n{2,}/)
    .map((p) => p.replace(/\s+\n/g, "\n").trim())
    .filter((p) => p.length > 0);
}

/** Human-readable file size (B / KB / MB / GB). Integers for bytes, one
 *  decimal otherwise. Used by the optional "Source documents" section. */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(
    units.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const value = bytes / 1024 ** i;
  const rounded = i === 0 ? Math.round(value) : Math.round(value * 10) / 10;
  return `${rounded} ${units[i]}`;
}

/* ------------------------------------------------------------------ */
/* Section building blocks                                            */
/* ------------------------------------------------------------------ */

function Section({
  children,
  wrap = true,
}: {
  children: ReactNode;
  wrap?: boolean;
}) {
  return (
    <View style={styles.section} wrap={wrap}>
      {children}
    </View>
  );
}

function Eyebrow({
  children,
  variant,
}: {
  children: ReactNode;
  variant?: "accent" | "danger";
}) {
  const variantStyle =
    variant === "accent"
      ? styles.eyebrowAccent
      : variant === "danger"
      ? styles.eyebrowDanger
      : null;
  return (
    <View style={styles.sectionHeader}>
      <Text style={variantStyle ? [styles.eyebrow, variantStyle] : styles.eyebrow}>
        {children}
      </Text>
    </View>
  );
}

/* ------------------------------------------------------------------ */
/* The document                                                       */
/* ------------------------------------------------------------------ */

export interface CaseVerdictPDFProps {
  detail: CaseDetail;
  t: TFunction;
  locale: string;
  /** Optional content extras (source documents, header note, generation
   *  details). Defaults to "everything off", so an export with no options
   *  passed is identical to the baseline document. */
  options?: PdfExportOptions;
  /** Override the "generated at" timestamp — defaults to now. Tests pass an
   *  explicit value so snapshots stay stable. */
  generatedAt?: Date;
}

export default function CaseVerdictPDF({
  detail,
  t,
  locale,
  options = DEFAULT_PDF_EXPORT_OPTIONS,
  generatedAt = new Date(),
}: CaseVerdictPDFProps) {
  const verdict = detail.verdict;
  const verdictRecord = detail.verdict_record;
  const review = detail.review;
  if (!verdict) {
    // Defensive: the caller already gates on detail.verdict, but render an
    // empty document rather than crash if it slips through.
    return (
      <Document>
        <Page size="A4" style={styles.page}>
          <Text style={styles.body}>{t("cases.no_verdict")}</Text>
        </Page>
      </Document>
    );
  }

  const generatedAtText = formatDateTime(generatedAt.toISOString(), locale);

  return (
    <Document
      title={`${t("cases.pdf.title")} — ${detail.case.patient_label || detail.case.id.slice(0, 8)}`}
      author="Conclave MD"
      creator="Conclave MD"
      producer="Conclave MD"
    >
      <Page size="A4" style={styles.page}>
        <Header
          detail={detail}
          verdictRecord={verdictRecord}
          t={t}
          locale={locale}
        />

        {nonEmptyText(options.headerNote) && (
          <Text style={styles.headerNote}>{options.headerNote.trim()}</Text>
        )}

        <Summary verdict={verdict} t={t} />
        <KeyClinicalData verdict={verdict} t={t} />
        <PrimaryRecommendation verdict={verdict} t={t} />
        <Certainty verdict={verdict} t={t} />
        <RedFlags verdict={verdict} t={t} />
        <FollowUp verdict={verdict} t={t} />
        <AppliedEvidence verdict={verdict} t={t} />
        {options.includeSourceFiles && (
          <SourceDocuments
            attachments={detail.attachments}
            showMeta={options.includeAttachmentMeta}
            t={t}
          />
        )}
        {options.includeGenerationMeta && verdictRecord && (
          <GenerationDetails
            verdictRecord={verdictRecord}
            t={t}
            locale={locale}
          />
        )}
        {review && <Review review={review} t={t} locale={locale} />}
        <Disclaimer t={t} />

        <Footer
          t={t}
          generatedAtText={generatedAtText}
          caseId={detail.case.id.slice(0, 8)}
        />
      </Page>
    </Document>
  );
}

/* ------------------------------------------------------------------ */
/* Header                                                             */
/* ------------------------------------------------------------------ */

function Header({
  detail,
  verdictRecord,
  t,
  locale,
}: {
  detail: CaseDetail;
  verdictRecord: VerdictRecord | null;
  t: TFunction;
  locale: string;
}) {
  const { case: caseRecord } = detail;
  const patientLabel = nonEmptyText(caseRecord.patient_label) ?? "—";
  const statusKey = caseRecord.status;
  const statusStyle = statusPalette[statusKey];

  const modelLine = verdictRecord
    ? `${verdictRecord.provider_id} · ${verdictRecord.model}`
    : "—";

  return (
    <View style={styles.header} wrap={false}>
      <Text style={styles.wordmark}>Conclave MD</Text>
      <Text style={styles.h1}>{t("cases.pdf.title")}</Text>
      <Text style={styles.patientLabel}>{patientLabel}</Text>
      <View style={styles.metaStrip}>
        <MetaItem
          label={t("cases.pdf.metadata.case_date")}
          value={formatDate(caseRecord.case_date || caseRecord.created_at, locale)}
        />
        <MetaItem
          label={t("cases.pdf.metadata.case_id")}
          value={caseRecord.id.slice(0, 8)}
          mono
        />
        <MetaItem label={t("cases.pdf.metadata.model")} value={modelLine} />
        <View style={styles.metaItem}>
          <Text style={styles.metaItemLabel}>
            {t("cases.pdf.metadata.status")}
          </Text>
          <View
            style={{
              backgroundColor: statusStyle.bg,
              paddingVertical: 2,
              paddingHorizontal: 6,
              borderRadius: 3,
            }}
          >
            <Text
              style={{
                fontFamily: "Helvetica-Bold",
                fontSize: 8,
                color: statusStyle.text,
                textTransform: "uppercase",
                letterSpacing: 0.4,
              }}
            >
              {statusLabel(t, statusKey)}
            </Text>
          </View>
        </View>
      </View>
      <View style={styles.headerRule} />
    </View>
  );
}

function MetaItem({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <View style={styles.metaItem}>
      <Text style={styles.metaItemLabel}>{label}</Text>
      <Text style={mono ? styles.metaItemValueMono : styles.metaItemValue}>
        {value}
      </Text>
    </View>
  );
}

/* ------------------------------------------------------------------ */
/* Body sections                                                      */
/* ------------------------------------------------------------------ */

function Summary({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  const paragraphs = splitParagraphs(verdict.case_summary);
  if (paragraphs.length === 0) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.case_summary")}</Eyebrow>
      {paragraphs.map((p, i) => (
        <Text
          key={i}
          style={[
            styles.body,
            i < paragraphs.length - 1 ? styles.paragraphSpacing : {},
          ]}
        >
          {p}
        </Text>
      ))}
    </Section>
  );
}

function KeyClinicalData({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (verdict.key_clinical_data.length === 0) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.key_clinical_data")}</Eyebrow>
      {verdict.key_clinical_data.map((kv, i) => (
        <View
          key={i}
          style={[
            styles.tableRow,
            i % 2 === 1 ? styles.tableRowZebra : {},
          ]}
          wrap={false}
        >
          <Text style={styles.tableLabel}>{kv.label}</Text>
          <Text style={styles.tableValue}>{kv.value}</Text>
        </View>
      ))}
    </Section>
  );
}

function PrimaryRecommendation({
  verdict,
  t,
}: {
  verdict: Verdict;
  t: TFunction;
}) {
  const rec = verdict.primary_recommendation;
  if (!nonEmptyText(rec.action) && !nonEmptyText(rec.rationale)) return null;
  return (
    <Section wrap={false}>
      <View style={styles.primaryBox}>
        <Text style={[styles.eyebrow, styles.eyebrowAccent]}>
          {t("cases.verdict.primary_recommendation")}
        </Text>
        {nonEmptyText(rec.action) && (
          <Text style={styles.primaryAction}>{rec.action}</Text>
        )}
        {nonEmptyText(rec.rationale) && (
          <Text style={styles.body}>{rec.rationale}</Text>
        )}
      </View>
    </Section>
  );
}

function Certainty({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  const level = verdict.certainty_level;
  const palette = certaintyPalette[level];
  return (
    <Section wrap={false}>
      <Eyebrow>{t("cases.verdict.certainty")}</Eyebrow>
      <View style={styles.certaintyRow}>
        <Text
          style={[
            styles.certaintyPill,
            { color: palette.text, backgroundColor: palette.bg },
          ]}
        >
          {t(`cases.pdf.certainty.${level}`)}
        </Text>
      </View>
      {nonEmptyText(verdict.certainty_justification) && (
        <Text style={styles.body}>{verdict.certainty_justification}</Text>
      )}
      {verdict.data_completeness && (
        <Text style={styles.body}>
          {`${t("cases.verdict.data_completeness")}: ${t(
            `cases.verdict.data_completeness_value.${verdict.data_completeness}`,
          )}`}
        </Text>
      )}
    </Section>
  );
}

function RedFlags({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (verdict.red_flags.length === 0) return null;
  return (
    <Section wrap={false}>
      <View style={styles.redFlagsBox}>
        <View style={{ marginBottom: 6 }}>
          <Text style={[styles.eyebrow, styles.eyebrowDanger]}>
            {t("cases.verdict.red_flags")}
          </Text>
        </View>
        {verdict.red_flags.map((flag, i) => (
          <View key={i} style={styles.bulletRow}>
            <Text style={styles.bulletMark}>■</Text>
            <Text style={[styles.body, { flex: 1 }]}>{flag}</Text>
          </View>
        ))}
      </View>
    </Section>
  );
}

function FollowUp({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (verdict.follow_up_triggers.length === 0) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.follow_up_triggers")}</Eyebrow>
      {verdict.follow_up_triggers.map((trigger, i) => (
        <View key={i} style={styles.bulletRow}>
          <Text style={styles.bulletMarkSubtle}>·</Text>
          <Text style={[styles.body, { flex: 1 }]}>{trigger}</Text>
        </View>
      ))}
    </Section>
  );
}

function AppliedEvidence({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (verdict.applied_evidence.length === 0) return null;
  return (
    <Section>
      <Eyebrow variant="accent">{t("cases.verdict.applied_evidence")}</Eyebrow>
      {verdict.applied_evidence.map((claim, i) => (
        <View key={i} style={styles.evidenceRow} wrap={false}>
          <Text style={styles.evidenceRef}>[{claim.ref}]</Text>
          <Text style={styles.evidenceClaim}>{claim.claim}</Text>
        </View>
      ))}
    </Section>
  );
}

/** Optional appendix: lists the case attachments by their original filename,
 *  keyed by the same `[A{position}]` ref the verdict cites in its applied
 *  evidence. Gated behind the `includeSourceFiles` export option. */
function SourceDocuments({
  attachments,
  showMeta,
  t,
}: {
  attachments: CaseAttachment[];
  showMeta: boolean;
  t: TFunction;
}) {
  if (attachments.length === 0) return null;
  const ordered = [...attachments].sort((a, b) => a.position - b.position);
  return (
    <Section>
      <Eyebrow>{t("cases.pdf.source_documents")}</Eyebrow>
      {ordered.map((a) => (
        <View key={a.id} style={styles.sourceRow} wrap={false}>
          <Text style={styles.sourceRef}>[A{a.position}]</Text>
          <Text style={styles.sourceName}>
            {a.original_filename}
            {showMeta && (
              <Text style={styles.sourceMeta}>
                {`   ${a.doc_type.toUpperCase()} · ${formatBytes(a.byte_size)}`}
              </Text>
            )}
          </Text>
        </View>
      ))}
    </Section>
  );
}

/** Optional appendix: provider/model and run telemetry from the verdict
 *  record. Gated behind the `includeGenerationMeta` export option. */
function GenerationDetails({
  verdictRecord,
  t,
  locale,
}: {
  verdictRecord: VerdictRecord;
  t: TFunction;
  locale: string;
}) {
  const rows: { label: string; value: string }[] = [
    {
      label: t("cases.pdf.metadata.model"),
      value: `${verdictRecord.provider_id} · ${verdictRecord.model}`,
    },
    {
      label: t("cases.pdf.gen_tokens"),
      value:
        verdictRecord.input_tokens + verdictRecord.output_tokens > 0
          ? `${verdictRecord.input_tokens} / ${verdictRecord.output_tokens}`
          : t("cases.pdf.gen_tokens_unreported"),
    },
    {
      label: t("cases.pdf.gen_latency"),
      value: `${(verdictRecord.latency_ms / 1000).toFixed(1)} s`,
    },
    {
      label: t("cases.pdf.gen_generated"),
      value: formatDateTime(verdictRecord.created_at, locale),
    },
  ];
  return (
    <Section wrap={false}>
      <Eyebrow>{t("cases.pdf.generation_details")}</Eyebrow>
      {rows.map((r, i) => (
        <View key={i} style={styles.genRow}>
          <Text style={styles.genLabel}>{r.label}</Text>
          <Text style={styles.genValue}>{r.value}</Text>
        </View>
      ))}
    </Section>
  );
}

function Review({
  review,
  t,
  locale,
}: {
  review: ReviewMetadataRecord;
  t: TFunction;
  locale: string;
}) {
  const palette = reviewPalette[review.decision];
  const reviewer = [review.reviewer_name, review.reviewer_role]
    .filter((s) => nonEmptyText(s))
    .join(" · ");
  return (
    <Section wrap={false}>
      <View style={styles.reviewBox}>
        <View style={{ flexDirection: "row", alignItems: "center", marginBottom: 4 }}>
          <Text style={styles.eyebrow}>{t("cases.pdf.review")}</Text>
          <Text
            style={[
              styles.certaintyPill,
              {
                color: palette.text,
                backgroundColor: palette.bg,
                marginLeft: 10,
              },
            ]}
          >
            {t(`cases.pdf.review_decision.${review.decision}`)}
          </Text>
        </View>
        {reviewer.length > 0 && (
          <Text style={styles.reviewMetaLine}>{reviewer}</Text>
        )}
        <Text style={styles.reviewMetaLine}>
          {formatDateTime(review.reviewed_at, locale)}
        </Text>
        {nonEmptyText(review.note) && (
          <Text style={styles.reviewNote}>“{review.note}”</Text>
        )}
      </View>
    </Section>
  );
}

/** The medical disclaimer is a fixed legal notice, not model output. Render
 *  it from the active locale so it always matches the UI language (the
 *  backend stores a canonical copy too, but display drives off i18n). */
function Disclaimer({ t }: { t: TFunction }) {
  const body = t("cases.verdict.disclaimer_body");
  if (!nonEmptyText(body)) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.disclaimer")}</Eyebrow>
      <View style={styles.disclaimerBox}>
        <Text style={styles.disclaimerText}>{body}</Text>
      </View>
    </Section>
  );
}

/* ------------------------------------------------------------------ */
/* Footer                                                             */
/* ------------------------------------------------------------------ */

// NOTE: the footer is intentionally 100% static. @react-pdf 4.5.1 silently
// drops any `fixed` element that uses a `render` callback once the page
// content flows across pages (verified by isolated repro) — that is exactly
// why the old `render`-based "Página X de Y" never painted. A static bar
// renders reliably on every page, so we show provenance + the case id (a
// stable per-page identifier) instead of a live page number.
function Footer({
  t,
  generatedAtText,
  caseId,
}: {
  t: TFunction;
  generatedAtText: string;
  caseId: string;
}) {
  return (
    <View style={styles.footer} fixed>
      <Text style={styles.footerText}>
        <Text style={styles.footerWordmark}>Conclave MD</Text>
        {` · ${t("cases.pdf.footer_generated", { date: generatedAtText })}`}
      </Text>
      <Text style={styles.footerId}>{caseId}</Text>
    </View>
  );
}
