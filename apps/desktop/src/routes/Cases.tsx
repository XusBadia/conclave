import { useEffect, useMemo, useState } from "react";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input, Textarea } from "../components/Field";
import {
  ipc,
  type CaseDetail,
  type CaseRecord,
  type ProviderInfo,
  type Verdict,
  type Workspace,
} from "../lib/ipc";

type View = "list" | "new" | "show";

export function CasesPage({ workspace }: { workspace: Workspace }) {
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
          title="Cases"
          subtitle={`${cases.length} stored in ${workspace.name}`}
          right={
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={refresh} loading={loading}>
                Refresh
              </Button>
              <Button
                size="sm"
                variant="primary"
                onClick={() => setView("new")}
              >
                New case
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
                No cases yet. Run your first virtual committee.
              </p>
              <div className="mt-4">
                <Button variant="primary" onClick={() => setView("new")}>
                  New case
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
                        {c.question || "(no question)"}
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
                      {c.status}
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
}: {
  workspace: Workspace;
  onCancel: () => void;
  onDone: (caseId: string) => void;
}) {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string>("");
  const [text, setText] = useState("");
  const [question, setQuestion] = useState("¿Cuál es el manejo recomendado?");
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

  const usable = useMemo(
    () => providers.filter((p) => p.configured || p.id === "ollama"),
    [providers],
  );

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
      setError("No provider configured");
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
          title="New case"
          subtitle="Paste a clinical note — it will be de-identified before any LLM call"
          right={
            <Button size="sm" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          }
        />
        <CardBody className="space-y-4">
          {error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          <Field label="Case text">
            <Textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={14}
              placeholder="Mujer de 67 años con disnea progresiva y edemas en MMII..."
            />
          </Field>
          <Field label="Question">
            <Input value={question} onChange={(e) => setQuestion(e.target.value)} />
          </Field>
          <Field label="Provider" hint="Configure providers under Settings">
            <select
              value={providerId}
              onChange={(e) => setProviderId(e.target.value)}
              className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink no-drag focus:outline-none focus:ring-conclave focus:border-accent"
            >
              {usable.length === 0 && <option value="">(no providers)</option>}
              {usable.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.id} · {p.default_model}
                </option>
              ))}
            </select>
          </Field>
          <div className="flex gap-2 pt-1">
            <Button onClick={previewDeident} disabled={!text.trim()}>
              Preview de-id
            </Button>
            <Button
              variant="primary"
              onClick={run}
              loading={busy}
              disabled={!text.trim() || !providerId}
            >
              Run committee
            </Button>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title="De-id preview"
          subtitle="Layer A regex + Layer C heuristics"
        />
        <CardBody>
          {preview ? (
            <div className="space-y-3">
              <div className="flex items-center gap-3 text-[12px]">
                <span className="rounded bg-surface px-2 py-0.5 text-ink-subtle">
                  {preview.spanCount} spans masked
                </span>
                <span
                  className={
                    preview.strictClean
                      ? "rounded bg-ok/15 px-2 py-0.5 text-ok"
                      : "rounded bg-warn/15 px-2 py-0.5 text-warn"
                  }
                >
                  strict {preview.strictClean ? "clean" : "dirty"}
                </span>
              </div>
              <pre className="max-h-[460px] overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-bg p-3 font-mono text-[12px] leading-relaxed text-ink-dim">
                {maskedPreview}
              </pre>
            </div>
          ) : (
            <p className="text-[13px] text-ink-subtle">
              Paste a case on the left and hit{" "}
              <span className="font-medium text-ink-dim">Preview de-id</span> to
              see exactly what will leave this device.
            </p>
          )}
        </CardBody>
      </Card>
    </div>
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
      alert(`Feedback "${kind}" recorded.`);
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
          ← Back to cases
        </Button>
        {detail.verdict && (
          <div className="flex gap-2">
            <Button size="sm" onClick={() => feedback("accept")} loading={busy}>
              Accept
            </Button>
            <Button size="sm" variant="ghost" onClick={() => feedback("modify")} loading={busy}>
              Mark modified
            </Button>
            <Button size="sm" variant="danger" onClick={() => feedback("reject")} loading={busy}>
              Reject
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
          title={detail.case.question || "(no question)"}
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
              No verdict on file for this case.
            </p>
          )}
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="De-identified case text" />
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
  const certaintyColor =
    verdict.certainty_level === "high"
      ? "text-ok"
      : verdict.certainty_level === "medium"
        ? "text-accent"
        : "text-warn";

  return (
    <div className="space-y-6">
      <section>
        <SectionTitle>Case summary</SectionTitle>
        <p>{verdict.case_summary}</p>
      </section>

      {verdict.key_clinical_data.length > 0 && (
        <section>
          <SectionTitle>Key clinical data</SectionTitle>
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
        <SectionTitle>Primary recommendation</SectionTitle>
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
          <SectionTitle>Alternatives</SectionTitle>
          <ul className="space-y-2">
            {verdict.alternatives.map((alt, i) => (
              <li
                key={i}
                className="rounded-md border border-border-subtle bg-bg px-3 py-2"
              >
                <div className="text-[13px] text-ink-dim">{alt.action}</div>
                <div className="mt-0.5 text-[12px] text-ink-faint">
                  when: {alt.when_to_consider}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}

      <section>
        <SectionTitle>Certainty</SectionTitle>
        <div className={`text-[14px] font-semibold ${certaintyColor}`}>
          {verdict.certainty_level.toUpperCase()}
        </div>
        <p className="mt-1">{verdict.certainty_justification}</p>
      </section>

      {verdict.red_flags.length > 0 && (
        <section>
          <SectionTitle>Red flags</SectionTitle>
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
          <SectionTitle>Follow-up triggers</SectionTitle>
          <ul className="list-inside list-disc space-y-1 text-[13px] text-ink-dim">
            {verdict.follow_up_triggers.map((t, i) => (
              <li key={i}>{t}</li>
            ))}
          </ul>
        </section>
      )}

      {verdict.applied_evidence.length > 0 && (
        <section>
          <SectionTitle>Applied evidence</SectionTitle>
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
        <SectionTitle>Disclaimer</SectionTitle>
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
