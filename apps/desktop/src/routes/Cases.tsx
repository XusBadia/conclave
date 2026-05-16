import { useEffect, useMemo, useState } from "react";
import { Trans, useTranslation } from "react-i18next";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input, Textarea } from "../components/Field";
import { cn } from "../lib/cn";
import {
  ipc,
  usableProviders,
  type CaseDetail,
  type CaseRecord,
  type ProviderInfo,
  type Verdict,
  type Workspace,
} from "../lib/ipc";
import { metaFor } from "../lib/providers";

type View = "list" | "new" | "show";

export function CasesPage({
  workspace,
  onGoToSettings,
}: {
  workspace: Workspace;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();
  const [view, setView] = useState<View>("list");
  const [cases, setCases] = useState<CaseRecord[]>([]);
  const [selected, setSelected] = useState<CaseDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setCases(await ipc.listCases(workspace.id, 50));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    setView("list");
    setSelected(null);
  }, [workspace.id]);

  if (view === "new") {
    return (
      <NewCase
        workspace={workspace}
        onCancel={() => setView("list")}
        onGoToSettings={onGoToSettings}
        onDone={async (id) => {
          await refresh();
          const det = await ipc.showCase(workspace.id, id);
          setSelected(det);
          setView("show");
        }}
      />
    );
  }

  if (view === "show" && selected) {
    return (
      <ShowCase
        workspace={workspace}
        detail={selected}
        onBack={() => {
          setSelected(null);
          setView("list");
        }}
      />
    );
  }

  return (
    <div className="mx-auto w-full max-w-5xl space-y-4 p-6">
      <Card>
        <CardHeader
          title={t("cases.page_title")}
          subtitle={t("cases.page_subtitle", {
            count: cases.length,
            workspace: workspace.name,
          })}
          right={
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={refresh} loading={loading}>
                {t("common.refresh")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                onClick={() => setView("new")}
              >
                {t("cases.new_button")}
              </Button>
            </div>
          }
        />
        <CardBody className="p-0">
          {error && (
            <div className="mx-5 mt-4 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          {cases.length === 0 && !loading && (
            <div className="px-6 py-12 text-center">
              <p className="text-[13px] text-ink-subtle">
                {t("cases.empty_title")}
              </p>
              <div className="mt-4">
                <Button variant="primary" onClick={() => setView("new")}>
                  {t("cases.new_button")}
                </Button>
              </div>
            </div>
          )}
          <ul className="divide-y divide-border-subtle">
            {cases.map((c) => (
              <li key={c.id}>
                <button
                  type="button"
                  onClick={async () => {
                    const det = await ipc.showCase(workspace.id, c.id);
                    setSelected(det);
                    setView("show");
                  }}
                  className="block w-full px-5 py-4 text-left transition hover:bg-surface no-drag focus:outline-none focus-visible:bg-surface"
                >
                  <div className="flex items-center justify-between gap-4">
                    <div className="min-w-0">
                      <div className="truncate text-[14px] font-medium text-ink">
                        {c.question || t("cases.no_question")}
                      </div>
                      <div className="mt-0.5 truncate text-[12px] text-ink-faint">
                        <span className="font-mono">{c.id}</span> · {new Date(
                          c.created_at,
                        ).toLocaleString()}
                      </div>
                    </div>
                    <span
                      className={
                        c.status === "completed"
                          ? "rounded bg-ok/15 px-2 py-0.5 text-[11px] font-medium text-ok"
                          : "rounded bg-danger/15 px-2 py-0.5 text-[11px] font-medium text-danger"
                      }
                    >
                      {t(`cases.status.${c.status}`)}
                    </span>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </CardBody>
      </Card>
    </div>
  );
}

function NewCase({
  workspace,
  onCancel,
  onDone,
  onGoToSettings,
}: {
  workspace: Workspace;
  onCancel: () => void;
  onDone: (caseId: string) => void;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [text, setText] = useState("");
  const [question, setQuestion] = useState(t("cases.default_question"));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [maskedPreview, setMaskedPreview] = useState<string | null>(null);
  const [preview, setPreview] = useState<{
    spanCount: number;
    strictClean: boolean;
  } | null>(null);

  useEffect(() => {
    (async () => {
      const ps = await ipc.listProviders();
      setProviders(ps);
      const first = ps.find((p) => p.configured || p.id === "ollama");
      if (first) setProviderId(first.id);
    })();
  }, []);

  const usable = useMemo(() => usableProviders(providers), [providers]);

  const previewDeident = async () => {
    if (!text.trim()) return;
    try {
      const r = await ipc.deidentText(text);
      setMaskedPreview(r.masked_text);
      setPreview({ spanCount: r.span_count, strictClean: r.strict_clean });
    } catch (e) {
      setError(String(e));
    }
  };

  const run = async () => {
    if (!text.trim()) return;
    if (!providerId) {
      setError(t("cases.no_provider_configured"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const resp = await ipc.runCase({
        workspace_id: workspace.id,
        text,
        question,
        provider_id: providerId,
      });
      onDone(resp.case.id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto grid w-full max-w-6xl grid-cols-1 gap-5 p-6 xl:grid-cols-[1fr,420px]">
      <Card>
        <CardHeader
          title={t("cases.new_title")}
          subtitle={t("cases.new_subtitle")}
          right={
            <Button size="sm" variant="ghost" onClick={onCancel}>
              {t("common.cancel")}
            </Button>
          }
        />
        <CardBody className="space-y-4">
          {error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          <Field label={t("cases.field_text")}>
            <Textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={14}
              placeholder={t("cases.field_text_placeholder")}
            />
          </Field>
          <Field label={t("cases.field_question")}>
            <Input value={question} onChange={(e) => setQuestion(e.target.value)} />
          </Field>
          <ProviderField
            providers={usable}
            providerId={providerId}
            onChange={setProviderId}
            onGoToSettings={onGoToSettings}
          />
          <div className="flex gap-2 pt-1">
            <Button onClick={previewDeident} disabled={!text.trim()}>
              {t("cases.preview_button")}
            </Button>
            <Button
              variant="primary"
              onClick={run}
              loading={busy}
              disabled={!text.trim() || !providerId}
            >
              {t("cases.run_button")}
            </Button>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title={t("cases.deid_title")}
          subtitle={t("cases.deid_subtitle")}
        />
        <CardBody>
          {preview ? (
            <div className="space-y-3">
              <div className="flex items-center gap-3 text-[12px]">
                <span className="rounded bg-surface px-2 py-0.5 text-ink-subtle">
                  {t("cases.deid_spans", { count: preview.spanCount })}
                </span>
                <span
                  className={
                    preview.strictClean
                      ? "rounded bg-ok/15 px-2 py-0.5 text-ok"
                      : "rounded bg-warn/15 px-2 py-0.5 text-warn"
                  }
                >
                  {preview.strictClean
                    ? t("cases.deid_strict_clean")
                    : t("cases.deid_strict_dirty")}
                </span>
              </div>
              <pre className="max-h-[460px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg p-3 font-mono text-[12px] leading-relaxed text-ink-dim">
                {maskedPreview}
              </pre>
            </div>
          ) : (
            <p className="text-[13px] text-ink-subtle">
              <Trans
                i18nKey="cases.deid_hint"
                components={[
                  <span key="0" className="font-medium text-ink-dim" />,
                ]}
              />
            </p>
          )}
        </CardBody>
      </Card>
    </div>
  );
}

// Provider field for the new-case form.
//
// Adapts to the single-active-provider rule:
//   • 0 usable → empty state with CTA back to Settings
//   • 1 usable → readonly summary chip + change link
//   • 2+      → labelled <select> with friendly names
function ProviderField({
  providers,
  providerId,
  onChange,
  onGoToSettings,
}: {
  providers: ProviderInfo[];
  providerId: string;
  onChange: (id: string) => void;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();

  if (providers.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border bg-bg-subtle p-4 text-center">
        <div className="text-[13.5px] font-medium text-ink">
          {t("cases.provider_empty_title")}
        </div>
        <p className="mx-auto mt-1 max-w-sm text-[12px] text-ink-subtle">
          {t("cases.provider_empty_body")}
        </p>
        {onGoToSettings && (
          <div className="mt-3">
            <Button size="sm" variant="primary" onClick={onGoToSettings}>
              {t("cases.provider_empty_cta")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (providers.length === 1) {
    const p = providers[0];
    const meta = metaFor(p.id);
    return (
      <Field label={t("cases.field_provider")}>
        <div
          className={cn(
            "flex items-center gap-3 rounded-lg border border-border bg-bg px-3 py-2.5",
          )}
        >
          <span
            aria-hidden
            className="grid h-8 w-8 shrink-0 place-content-center rounded-md bg-slate-400/10 text-[12px] font-semibold text-ink-dim ring-1 ring-border-subtle"
          >
            {meta.monogram}
          </span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13px] font-medium text-ink">
              {meta.name}
            </div>
            <div className="truncate text-[11.5px] text-ink-faint">
              <span className="font-mono">{p.default_model}</span>
              {" · "}
              {meta.authLabel}
            </div>
          </div>
          {onGoToSettings && (
            <button
              type="button"
              onClick={onGoToSettings}
              className="rounded-md px-2 py-1 text-[12px] text-ink-subtle transition no-drag hover:bg-surface hover:text-ink focus:outline-none focus-visible:ring-conclave"
            >
              {t("cases.provider_change_link")}
            </button>
          )}
        </div>
      </Field>
    );
  }

  return (
    <Field
      label={t("cases.field_provider")}
      hint={onGoToSettings ? undefined : t("cases.field_provider_hint")}
    >
      <select
        value={providerId}
        onChange={(e) => onChange(e.target.value)}
        className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink no-drag focus:outline-none focus:ring-conclave focus:border-accent"
      >
        {providers.map((p) => {
          const meta = metaFor(p.id);
          return (
            <option key={p.id} value={p.id}>
              {meta.name} · {p.default_model}
            </option>
          );
        })}
      </select>
      {onGoToSettings && (
        <button
          type="button"
          onClick={onGoToSettings}
          className="mt-1.5 text-[12px] text-ink-faint transition no-drag hover:text-ink focus:outline-none focus-visible:underline"
        >
          {t("cases.provider_change_link")}
        </button>
      )}
    </Field>
  );
}

function ShowCase({
  workspace,
  detail,
  onBack,
}: {
  workspace: Workspace;
  detail: CaseDetail;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const feedback = async (kind: "accept" | "modify" | "reject") => {
    setBusy(true);
    setError(null);
    try {
      await ipc.submitFeedback({
        workspace_id: workspace.id,
        case_id: detail.case.id,
        kind,
      });
      alert(t("cases.feedback_recorded", { kind }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto w-full max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between">
        <Button size="sm" variant="ghost" onClick={onBack}>
          {t("cases.back")}
        </Button>
        {detail.verdict && (
          <div className="flex gap-2">
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

      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
          {error}
        </div>
      )}

      <Card>
        <CardHeader
          title={detail.case.question || t("cases.no_question")}
          subtitle={`${detail.case.id} · ${new Date(detail.case.created_at).toLocaleString()}`}
          right={
            detail.verdict_record && (
              <span className="text-[12px] text-ink-faint">
                {detail.verdict_record.provider_id} · {detail.verdict_record.model} ·
                {" "}
                {detail.verdict_record.latency_ms}ms
              </span>
            )
          }
        />
        <CardBody className="space-y-6 prose-conclave">
          {detail.verdict ? (
            <VerdictRenderer verdict={detail.verdict} />
          ) : (
            <p className="text-[13px] text-ink-subtle">
              {t("cases.no_verdict")}
            </p>
          )}
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

function VerdictRenderer({ verdict }: { verdict: Verdict }) {
  const { t } = useTranslation();
  const certaintyColor =
    verdict.certainty_level === "high"
      ? "text-ok"
      : verdict.certainty_level === "medium"
        ? "text-accent"
        : "text-warn";

  return (
    <div className="space-y-6">
      <section>
        <SectionTitle>{t("cases.verdict.case_summary")}</SectionTitle>
        <p>{verdict.case_summary}</p>
      </section>

      {verdict.key_clinical_data.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.key_clinical_data")}</SectionTitle>
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
        </section>
      )}

      <section>
        <SectionTitle>{t("cases.verdict.primary_recommendation")}</SectionTitle>
        <div className="rounded-lg border border-accent/30 bg-accent/5 px-4 py-3">
          <div className="text-[14px] font-semibold text-ink">
            {verdict.primary_recommendation.action}
          </div>
          <div className="mt-1 text-[13px] text-ink-dim">
            {verdict.primary_recommendation.rationale}
          </div>
        </div>
      </section>

      {verdict.alternatives.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.alternatives")}</SectionTitle>
          <ul className="space-y-2">
            {verdict.alternatives.map((alt, i) => (
              <li
                key={i}
                className="rounded-md border border-border-subtle bg-bg px-3 py-2"
              >
                <div className="text-[13px] text-ink-dim">{alt.action}</div>
                <div className="mt-0.5 text-[12px] text-ink-faint">
                  {t("cases.verdict.alternative_when", {
                    when: alt.when_to_consider,
                  })}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <SectionTitle>{t("cases.verdict.certainty")}</SectionTitle>
        <div className={`text-[14px] font-semibold ${certaintyColor}`}>
          {verdict.certainty_level.toUpperCase()}
        </div>
        <p className="mt-1">{verdict.certainty_justification}</p>
      </section>

      {verdict.red_flags.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.red_flags")}</SectionTitle>
          <ul className="space-y-1.5">
            {verdict.red_flags.map((rf, i) => (
              <li
                key={i}
                className="rounded-md border border-warn/40 bg-warn/5 px-3 py-2 text-[13px] text-ink-dim"
              >
                ⚠ {rf}
              </li>
            ))}
          </ul>
        </section>
      )}

      {verdict.follow_up_triggers.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.follow_up_triggers")}</SectionTitle>
          <ul className="list-inside list-disc space-y-1 text-[13px] text-ink-dim">
            {verdict.follow_up_triggers.map((tr, i) => (
              <li key={i}>{tr}</li>
            ))}
          </ul>
        </section>
      )}

      {verdict.applied_evidence.length > 0 && (
        <section>
          <SectionTitle>{t("cases.verdict.applied_evidence")}</SectionTitle>
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
        </section>
      )}

      <section>
        <SectionTitle>{t("cases.verdict.disclaimer")}</SectionTitle>
        <p className="text-[12px] leading-relaxed text-ink-subtle">
          {verdict.disclaimer}
        </p>
      </section>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-1.5 text-[11px] uppercase tracking-[0.08em] text-ink-faint">
      {children}
    </h4>
  );
}
