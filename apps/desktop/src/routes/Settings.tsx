import { useEffect, useState } from "react";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input } from "../components/Field";
import { ipc, type ProviderInfo } from "../lib/ipc";

export function SettingsPage() {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);
  const [testOutput, setTestOutput] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setProviders(await ipc.listProviders());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const saveKey = async (id: string) => {
    const key = drafts[id] ?? "";
    if (!key.trim()) return;
    setBusy({ ...busy, [id]: true });
    setError(null);
    try {
      await ipc.setProviderKey(id, key);
      setDrafts({ ...drafts, [id]: "" });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy({ ...busy, [id]: false });
    }
  };

  const testKey = async (id: string) => {
    setBusy({ ...busy, [id]: true });
    setError(null);
    setTestOutput(null);
    try {
      const out = await ipc.testProvider(id);
      setTestOutput(`${id}\n\n${out}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy({ ...busy, [id]: false });
    }
  };

  const removeKey = async (id: string) => {
    if (!confirm(`Remove API key for ${id}?`)) return;
    setBusy({ ...busy, [id]: true });
    try {
      await ipc.removeProviderKey(id);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy({ ...busy, [id]: false });
    }
  };

  return (
    <div className="mx-auto w-full max-w-3xl space-y-5 p-6">
      <Card>
        <CardHeader
          title="LLM providers"
          subtitle="API keys live in your OS keychain · never in config files"
          right={
            <Button size="sm" variant="ghost" onClick={refresh} loading={loading}>
              Refresh
            </Button>
          }
        />
        <CardBody className="space-y-4">
          {error && (
            <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          {providers.map((p) => (
            <div
              key={p.id}
              className="rounded-lg border border-border-subtle bg-bg p-4"
            >
              <div className="mb-3 flex items-center justify-between">
                <div>
                  <div className="text-[14px] font-semibold text-ink">
                    {p.id}
                  </div>
                  <div className="mt-0.5 text-[12px] text-ink-faint">
                    default model:{" "}
                    <span className="font-mono text-ink-subtle">
                      {p.default_model}
                    </span>
                    {p.requires_network ? " · network" : " · local"}
                  </div>
                </div>
                <div className="flex items-center gap-1.5 text-[12px]">
                  <span
                    className={`h-1.5 w-1.5 rounded-full ${
                      p.configured ? "bg-ok" : "bg-ink-faint"
                    }`}
                  />
                  <span className="text-ink-subtle">
                    {p.configured ? "configured" : "not configured"}
                  </span>
                  <span className="mx-1 text-ink-faint">·</span>
                  <span
                    className={`h-1.5 w-1.5 rounded-full ${
                      p.available ? "bg-ok" : "bg-warn"
                    }`}
                  />
                  <span className="text-ink-subtle">
                    {p.available ? "reachable" : "unreachable"}
                  </span>
                </div>
              </div>

              {p.id !== "ollama" && (
                <div className="flex items-end gap-2">
                  <Field label="API key">
                    <Input
                      type="password"
                      value={drafts[p.id] ?? ""}
                      onChange={(e) =>
                        setDrafts({ ...drafts, [p.id]: e.target.value })
                      }
                      placeholder="paste here · stored in keychain"
                    />
                  </Field>
                  <Button
                    size="md"
                    variant="primary"
                    onClick={() => saveKey(p.id)}
                    loading={busy[p.id]}
                    disabled={!(drafts[p.id] ?? "").trim()}
                  >
                    Save
                  </Button>
                </div>
              )}
              <div className="mt-3 flex items-center gap-2">
                <Button
                  size="sm"
                  onClick={() => testKey(p.id)}
                  loading={busy[p.id]}
                  disabled={!p.configured && p.id !== "ollama"}
                >
                  Test
                </Button>
                {p.configured && p.id !== "ollama" && (
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => removeKey(p.id)}
                  >
                    Remove key
                  </Button>
                )}
              </div>
            </div>
          ))}
        </CardBody>
      </Card>

      {testOutput && (
        <Card>
          <CardHeader title="Last test" />
          <CardBody>
            <pre className="whitespace-pre-wrap font-mono text-[12px] leading-relaxed text-ink-dim">
              {testOutput}
            </pre>
          </CardBody>
        </Card>
      )}
    </div>
  );
}
