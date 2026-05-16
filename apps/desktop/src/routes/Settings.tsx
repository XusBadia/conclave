import { useEffect, useMemo, useState } from "react";

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

  const standard = useMemo(
    () => providers.filter((p) => p.kind === "standard"),
    [providers],
  );
  const oauth = useMemo(
    () => providers.filter((p) => p.kind === "oauth"),
    [providers],
  );

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
          title="LLM providers — API key"
          subtitle="Keys live in your OS keychain · never in config files"
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
          {standard.map((p) => (
            <ProviderRow
              key={p.id}
              provider={p}
              draft={drafts[p.id] ?? ""}
              setDraft={(v) => setDrafts({ ...drafts, [p.id]: v })}
              busy={!!busy[p.id]}
              onSave={() => saveKey(p.id)}
              onTest={() => testKey(p.id)}
              onRemove={() => removeKey(p.id)}
            />
          ))}
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title="LLM providers — subscription (OAuth)"
          subtitle="Reuse the credentials dropped by `claude login` / `codex login`"
          right={
            <span className="rounded bg-warn/15 px-2 py-0.5 text-[11px] font-medium text-warn">
              experimental
            </span>
          }
        />
        <CardBody className="space-y-4">
          <p className="text-[12px] leading-relaxed text-ink-faint">
            These backends piggyback on the official CLIs' OAuth tokens —
            endpoints are undocumented and may break. Make sure you have
            authenticated each CLI first; Conclave will detect the
            credentials file automatically.
          </p>
          {oauth.map((p) => (
            <OAuthRow
              key={p.id}
              provider={p}
              busy={!!busy[p.id]}
              onTest={() => testKey(p.id)}
              onRefresh={refresh}
              onChange={refresh}
            />
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

function ProviderRow({
  provider: p,
  draft,
  setDraft,
  busy,
  onSave,
  onTest,
  onRemove,
}: {
  provider: ProviderInfo;
  draft: string;
  setDraft: (v: string) => void;
  busy: boolean;
  onSave: () => void;
  onTest: () => void;
  onRemove: () => void;
}) {
  return (
    <div className="rounded-lg border border-border-subtle bg-bg p-4">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <div className="text-[14px] font-semibold text-ink">{p.id}</div>
          <div className="mt-0.5 text-[12px] text-ink-faint">
            default model:{" "}
            <span className="font-mono text-ink-subtle">{p.default_model}</span>
            {p.requires_network ? " · network" : " · local"}
          </div>
        </div>
        <StatusPills configured={p.configured} available={p.available} />
      </div>

      {p.id !== "ollama" && (
        <div className="flex items-end gap-2">
          <Field label="API key">
            <Input
              type="password"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="paste here · stored in keychain"
            />
          </Field>
          <Button
            size="md"
            variant="primary"
            onClick={onSave}
            loading={busy}
            disabled={!draft.trim()}
          >
            Save
          </Button>
        </div>
      )}
      <div className="mt-3 flex items-center gap-2">
        <Button
          size="sm"
          onClick={onTest}
          loading={busy}
          disabled={!p.configured && p.id !== "ollama"}
        >
          Test
        </Button>
        {p.configured && p.id !== "ollama" && (
          <Button size="sm" variant="danger" onClick={onRemove}>
            Remove key
          </Button>
        )}
      </div>
    </div>
  );
}

function OAuthRow({
  provider: p,
  busy,
  onTest,
  onRefresh,
  onChange,
}: {
  provider: ProviderInfo;
  busy: boolean;
  onTest: () => void;
  onRefresh: () => void;
  onChange: () => void;
}) {
  const [signingIn, setSigningIn] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pasteCode, setPasteCode] = useState<string | null>(null);
  const [code, setCode] = useState("");

  const signInAnthropic = async () => {
    setError(null);
    setSigningIn(true);
    try {
      const r = await ipc.oauthAnthropicStart();
      setPasteCode(r.instructions);
    } catch (e) {
      setError(String(e));
    } finally {
      setSigningIn(false);
    }
  };

  const submitCode = async () => {
    if (!code.trim()) return;
    setError(null);
    setSigningIn(true);
    try {
      await ipc.oauthAnthropicComplete(code.trim());
      setPasteCode(null);
      setCode("");
      onChange();
    } catch (e) {
      setError(String(e));
    } finally {
      setSigningIn(false);
    }
  };

  const signInOpenAI = async () => {
    setError(null);
    setSigningIn(true);
    try {
      await ipc.oauthOpenaiLogin();
      onChange();
    } catch (e) {
      setError(String(e));
    } finally {
      setSigningIn(false);
    }
  };

  const logout = async () => {
    if (!confirm(`Sign out of ${p.id}?`)) return;
    setError(null);
    try {
      await ipc.oauthLogout(p.id);
      onChange();
    } catch (e) {
      setError(String(e));
    }
  };

  const friendly =
    p.id === "anthropic-oauth"
      ? "Anthropic — Claude Max subscription"
      : "OpenAI — ChatGPT subscription";

  return (
    <div className="rounded-lg border border-border-subtle bg-bg p-4">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <div className="text-[14px] font-semibold text-ink">{friendly}</div>
          <div className="mt-0.5 text-[12px] text-ink-faint">
            default model:{" "}
            <span className="font-mono text-ink-subtle">{p.default_model}</span>
            {p.hint && (
              <>
                {" · "}
                <span className="text-ink-subtle">{p.hint}</span>
              </>
            )}
          </div>
        </div>
        <StatusPills configured={p.configured} available={p.available} />
      </div>

      {error && (
        <div className="mb-3 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[12px] text-danger">
          {error}
        </div>
      )}

      {pasteCode && (
        <div className="mb-3 space-y-2 rounded-md border border-accent/30 bg-accent/5 p-3">
          <p className="text-[12px] text-ink-dim">{pasteCode}</p>
          <Field label="One-time code">
            <Input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              placeholder="paste here (format: code#state)"
            />
          </Field>
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="primary"
              onClick={submitCode}
              loading={signingIn}
              disabled={!code.trim()}
            >
              Finish sign in
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setPasteCode(null);
                setCode("");
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {!p.configured && !pasteCode && (
          <Button
            size="sm"
            variant="primary"
            onClick={
              p.id === "anthropic-oauth" ? signInAnthropic : signInOpenAI
            }
            loading={signingIn || busy}
          >
            Sign in with browser
          </Button>
        )}
        {p.configured && (
          <>
            <Button size="sm" onClick={onTest} loading={busy}>
              Test
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={
                p.id === "anthropic-oauth" ? signInAnthropic : signInOpenAI
              }
              loading={signingIn}
            >
              Re-sign in
            </Button>
            <Button size="sm" variant="danger" onClick={logout}>
              Sign out
            </Button>
          </>
        )}
        <Button size="sm" variant="ghost" onClick={onRefresh}>
          Refresh
        </Button>
      </div>
    </div>
  );
}

function StatusPills({
  configured,
  available,
}: {
  configured: boolean;
  available: boolean;
}) {
  return (
    <div className="flex items-center gap-1.5 text-[12px]">
      <span
        className={`h-1.5 w-1.5 rounded-full ${
          configured ? "bg-ok" : "bg-ink-faint"
        }`}
      />
      <span className="text-ink-subtle">
        {configured ? "configured" : "not configured"}
      </span>
      <span className="mx-1 text-ink-faint">·</span>
      <span
        className={`h-1.5 w-1.5 rounded-full ${
          available ? "bg-ok" : "bg-warn"
        }`}
      />
      <span className="text-ink-subtle">
        {available ? "reachable" : "unreachable"}
      </span>
    </div>
  );
}
