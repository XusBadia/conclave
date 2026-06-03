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
  CaseDetail,
  CaseRecord,
  ReviewMetadataRecord,
  Verdict,
  VerdictRecord,
} from "../lib/ipc";

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
    fontSize: 10.5,
    color: color.inkDim,
    lineHeight: 1.5,
  },

  /* Header */
  header: { marginBottom: 24 },
  headerRow: { flexDirection: "row", alignItems: "flex-start" },
  headerLeft: { flexGrow: 1, paddingRight: 16 },
  headerRight: { width: 200, alignItems: "flex-end" },
  wordmark: {
    fontFamily: "Helvetica-Bold",
    fontSize: 11,
    color: color.ink,
    letterSpacing: 1.2,
    textTransform: "uppercase",
    marginBottom: 10,
  },
  h1: {
    fontFamily: "Helvetica-Bold",
    fontSize: 18,
    color: color.ink,
    lineHeight: 1.25,
  },
  patientLabel: {
    fontFamily: "Helvetica-Bold",
    fontSize: 11,
    color: color.inkDim,
    marginTop: 6,
  },
  metaPair: { flexDirection: "row", marginBottom: 4 },
  metaLabel: {
    fontFamily: "Helvetica",
    fontSize: 8.5,
    color: color.inkSubtle,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    width: 70,
    textAlign: "right",
    marginRight: 8,
  },
  metaValue: {
    fontFamily: "Helvetica",
    fontSize: 9.5,
    color: color.ink,
    textAlign: "right",
  },
  metaValueMono: {
    fontFamily: "Courier",
    fontSize: 9,
    color: color.ink,
    textAlign: "right",
  },
  headerRule: {
    marginTop: 16,
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairlineStrong,
  },

  /* Section primitives */
  section: { marginBottom: 20 },
  sectionHeader: { marginBottom: 8 },
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
    fontSize: 10.5,
    color: color.inkDim,
    lineHeight: 1.5,
  },
  bodyStrong: {
    fontFamily: "Helvetica-Bold",
    fontSize: 10.5,
    color: color.ink,
  },
  paragraphSpacing: { marginBottom: 6 },

  /* Clinical data table */
  tableRow: {
    flexDirection: "row",
    borderBottomWidth: 0.5,
    borderBottomColor: color.hairline,
    paddingVertical: 8,
    paddingHorizontal: 4,
  },
  tableRowZebra: { backgroundColor: color.surfaceSoft },
  tableLabel: {
    width: "35%",
    fontFamily: "Helvetica-Bold",
    fontSize: 10,
    color: color.ink,
    paddingRight: 10,
  },
  tableValue: {
    width: "65%",
    fontFamily: "Helvetica",
    fontSize: 10,
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
    fontSize: 13,
    color: color.ink,
    marginTop: 6,
    marginBottom: 8,
    lineHeight: 1.35,
  },

  /* Alternatives */
  altRow: { flexDirection: "row", marginBottom: 10 },
  altIndex: {
    width: 18,
    fontFamily: "Courier",
    fontSize: 10,
    color: color.inkSubtle,
  },
  altBody: { flex: 1 },
  altWhen: {
    fontFamily: "Helvetica-Oblique",
    fontSize: 9.5,
    color: color.inkSubtle,
    marginTop: 2,
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
  evidenceClaim: { flex: 1, fontSize: 10, color: color.inkDim },

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
    fontSize: 10,
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

  /* Footer */
  footer: {
    position: "absolute",
    bottom: 28,
    left: 52,
    right: 52,
    borderTopWidth: 0.5,
    borderTopColor: color.hairline,
    paddingTop: 8,
    flexDirection: "row",
    justifyContent: "space-between",
    fontFamily: "Helvetica",
    fontSize: 8.5,
    color: color.inkFaint,
  },
  footerWordmark: {
    fontFamily: "Helvetica-Bold",
    fontSize: 8.5,
    color: color.inkFaint,
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
  /** Override the "generated at" timestamp — defaults to now. Tests pass an
   *  explicit value so snapshots stay stable. */
  generatedAt?: Date;
}

export default function CaseVerdictPDF({
  detail,
  t,
  locale,
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
      author="Conclave"
      creator="Conclave"
      producer="Conclave"
    >
      <Page size="A4" style={styles.page}>
        <Header
          detail={detail}
          verdictRecord={verdictRecord}
          t={t}
          locale={locale}
        />

        <Summary verdict={verdict} t={t} />
        <KeyClinicalData verdict={verdict} t={t} />
        <PrimaryRecommendation verdict={verdict} t={t} />
        <Alternatives verdict={verdict} t={t} />
        <Certainty verdict={verdict} t={t} />
        <RedFlags verdict={verdict} t={t} />
        <FollowUp verdict={verdict} t={t} />
        <AppliedEvidence verdict={verdict} t={t} />
        {review && <Review review={review} t={t} locale={locale} />}
        <Disclaimer verdict={verdict} t={t} />

        <Footer t={t} generatedAtText={generatedAtText} />
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
      <View style={styles.headerRow}>
        <View style={styles.headerLeft}>
          <Text style={styles.wordmark}>Conclave</Text>
          <Text style={styles.h1}>{t("cases.pdf.title")}</Text>
          <Text style={styles.patientLabel}>{patientLabel}</Text>
        </View>
        <View style={styles.headerRight}>
          <MetaPair
            label={t("cases.pdf.metadata.case_date")}
            value={formatDate(caseRecord.case_date || caseRecord.created_at, locale)}
          />
          <MetaPair
            label={t("cases.pdf.metadata.case_id")}
            value={caseRecord.id.slice(0, 8)}
            mono
          />
          <MetaPair label={t("cases.pdf.metadata.model")} value={modelLine} />
          <View style={styles.metaPair}>
            <Text style={styles.metaLabel}>{t("cases.pdf.metadata.status")}</Text>
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
      </View>
      <View style={styles.headerRule} />
    </View>
  );
}

function MetaPair({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <View style={styles.metaPair}>
      <Text style={styles.metaLabel}>{label}</Text>
      <Text style={mono ? styles.metaValueMono : styles.metaValue}>{value}</Text>
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

function Alternatives({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (verdict.alternatives.length === 0) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.alternatives")}</Eyebrow>
      {verdict.alternatives.map((alt, i) => (
        <View key={i} style={styles.altRow} wrap={false}>
          <Text style={styles.altIndex}>{i + 1}.</Text>
          <View style={styles.altBody}>
            <Text style={styles.bodyStrong}>{alt.action}</Text>
            {nonEmptyText(alt.when_to_consider) && (
              <Text style={styles.altWhen}>
                {t("cases.verdict.alternative_when", { when: alt.when_to_consider })}
              </Text>
            )}
          </View>
        </View>
      ))}
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

function Disclaimer({ verdict, t }: { verdict: Verdict; t: TFunction }) {
  if (!nonEmptyText(verdict.disclaimer)) return null;
  return (
    <Section>
      <Eyebrow>{t("cases.verdict.disclaimer")}</Eyebrow>
      <View style={styles.disclaimerBox}>
        <Text style={styles.disclaimerText}>{verdict.disclaimer}</Text>
      </View>
    </Section>
  );
}

/* ------------------------------------------------------------------ */
/* Footer                                                             */
/* ------------------------------------------------------------------ */

function Footer({
  t,
  generatedAtText,
}: {
  t: TFunction;
  generatedAtText: string;
}) {
  return (
    <View style={styles.footer} fixed>
      <Text>
        <Text style={styles.footerWordmark}>Conclave</Text>
        <Text> · {t("cases.pdf.footer_generated", { date: generatedAtText })}</Text>
      </Text>
      <Text
        render={({ pageNumber, totalPages }) =>
          t("cases.pdf.footer_page", { page: pageNumber, total: totalPages })
        }
      />
    </View>
  );
}
