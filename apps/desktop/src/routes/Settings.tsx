import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input } from "../components/Field";
import { cn } from "../lib/cn";
import { getLocale, setLocale, type Locale } from "../i18n";
import { useTheme } from "../lib/theme";
import {
  activeProvider,
  connectedSlotProviders,
  ipc,
  type ProviderInfo,
} from "../lib/ipc";
import {
  BRAND_HOVER,
  BRAND_TINT,
  PICKER_GROUPS,
  metaFor,
  type ProviderMeta,
} from "../lib/providers";

// ---------------------------------------------------------------------------
// State machine for the connect overlay (shown inside the main card when the
// user picks a tile from the picker).
// ---------------------------------------------------------------------------
type ConnectFlow =
  | { kind: "idle" }
  | { kind: "api-key"; id: string; draft: string }
  | { kind: "oauth-anthropic"; pasteInstructions: string | null; code: string }
  | { kind: "oauth-openai" };

export function SettingsPage() {
  const { t } = useTranslation();

  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [flow, setFlow] = useState<ConnectFlow>({ kind: "idle" });
  const [migrationOpen, setMigrationOpen] = useState(false);
  const migrationDismissed = useRef(false);

  const active = useMemo(() => activeProvider(providers), [providers]);
  const ollama = useMemo(
    () => providers.find((p) => p.id === "ollama"),
    [providers],
  );

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await ipc.listProviders();
      setProviders(list);
      if (
        connectedSlotProviders(list).length > 1 &&
        !migrationDismissed.current
      ) {
        setMigrationOpen(true);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  // ---- Actions ------------------------------------------------------------

  const saveApiKey = async (id: string, draft: string) => {
    if (!draft.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.setProviderKey(id, draft);
      setFlow({ kind: "idle" });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const startAnthropicOAuth = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await ipc.oauthAnthropicStart();
      setFlow({
        kind: "oauth-anthropic",
        pasteInstructions: r.instructions,
        code: "",
      });
    } catch (e) {
      setError(String(e));
      setFlow({ kind: "idle" });
    } finally {
      setBusy(false);
    }
  };

  const submitAnthropicCode = async (code: string) => {
    if (!code.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.oauthAnthropicComplete(code.trim());
      setFlow({ kind: "idle" });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const startOpenAIOAuth = async () => {
    setFlow({ kind: "oauth-openai" });
    setBusy(true);
    setError(null);
    try {
      await ipc.oauthOpenaiLogin();
      setFlow({ kind: "idle" });
      await refresh();
    } catch (e) {
      setError(String(e));
      setFlow({ kind: "idle" });
    } finally {
      setBusy(false);
    }
  };

  const pickProvider = (id: string) => {
    const meta = metaFor(id);
    if (id === "anthropic-oauth") return startAnthropicOAuth();
    if (id === "openai-oauth") return startOpenAIOAuth();
    if (meta.authLabel === "API key") {
      setFlow({ kind: "api-key", id, draft: "" });
    }
  };

  const testActive = async (id: string) => {
    setBusy(true);
    setError(null);
    setTestOutput(null);
    try {
      const out = await ipc.testProvider(id);
      setTestOutput(`${metaFor(id).name}\n\n${out}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const disconnect = async (p: ProviderInfo, opts: { confirm: boolean }) => {
    if (
      opts.confirm &&
      !window.confirm(
        t("settings.confirm_disconnect", { name: metaFor(p.id).name }),
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (p.auth === "oauth") await ipc.oauthLogout(p.id);
      else await ipc.removeProviderKey(p.id);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const switchProvider = async (p: ProviderInfo) => {
    if (
      !window.confirm(
        t("settings.confirm_switch", { name: metaFor(p.id).name }),
      )
    ) {
      return;
    }
    await disconnect(p, { confirm: false });
  };

  const completeMigration = async (keepId: string) => {
    const toDisconnect = connectedSlotProviders(providers).filter(
      (p) => p.id !== keepId,
    );
    setBusy(true);
    setError(null);
    const errs: string[] = [];
    for (const p of toDisconnect) {
      try {
        if (p.auth === "oauth") await ipc.oauthLogout(p.id);
        else await ipc.removeProviderKey(p.id);
      } catch (e) {
        errs.push(`${metaFor(p.id).name}: ${String(e)}`);
      }
    }
    if (errs.length) setError(errs.join(" · "));
    setMigrationOpen(false);
    migrationDismissed.current = true;
    setBusy(false);
    await refresh();
  };

  const dismissMigration = () => {
    setMigrationOpen(false);
    migrationDismissed.current = true;
  };

  // ---- Render -------------------------------------------------------------

  return (
    <div className="mx-auto w-full max-w-2xl space-y-5 p-6">
      <LanguageCard />

      <Card>
        <CardHeader
          title={t("settings.ai_title")}
          subtitle={
            active
              ? t("settings.ai_subtitle_active")
              : t("settings.ai_subtitle_empty")
          }
          right={
            <Button
              size="sm"
              variant="ghost"
              onClick={refresh}
              loading={loading}
            >
              {t("settings.refresh")}
            </Button>
          }
        />
        <CardBody className="space-y-4">
          {error && (
            <div
              role="alert"
              className="animate-in rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
            >
              {error}
            </div>
          )}

          {flow.kind !== "idle" ? (
            <ConnectFlowView
              flow={flow}
              busy={busy}
              onCancel={() => setFlow({ kind: "idle" })}
              onSaveApiKey={(draft) =>
                flow.kind === "api-key" && saveApiKey(flow.id, draft)
              }
              onSubmitAnthropicCode={submitAnthropicCode}
              onUpdateAnthropicCode={(code) =>
                flow.kind === "oauth-anthropic" &&
                setFlow({ ...flow, code })
              }
              onUpdateApiDraft={(draft) =>
                flow.kind === "api-key" && setFlow({ ...flow, draft })
              }
            />
          ) : active ? (
            <ActiveProviderView
              provider={active}
              busy={busy}
              ollamaAvailable={!!ollama?.available}
              onTest={() => testActive(active.id)}
              onSwitch={() => switchProvider(active)}
              onDisconnect={() => disconnect(active, { confirm: true })}
            />
          ) : (
            <ProviderPicker
              providers={providers}
              busy={busy}
              ollamaAvailable={!!ollama?.available}
              onPick={pickProvider}
            />
          )}
        </CardBody>
      </Card>

      {testOutput && (
        <Card>
          <CardHeader
            title={t("settings.test_output_title")}
            right={
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setTestOutput(null)}
              >
                {t("settings.test_output_close")}
              </Button>
            }
          />
          <CardBody>
            <pre className="whitespace-pre-wrap font-mono text-[12px] leading-relaxed text-ink-dim">
              {testOutput}
            </pre>
          </CardBody>
        </Card>
      )}

      <p className="px-1 text-center text-[11px] text-ink-faint">
        {t("settings.keychain_note")}
      </p>

      {migrationOpen && (
        <MigrationDialog
          candidates={connectedSlotProviders(providers)}
          busy={busy}
          onKeep={completeMigration}
          onCancel={dismissMigration}
        />
      )}
    </div>
  );
}

// ===========================================================================
// Language switcher
// ===========================================================================
function LanguageCard() {
  const { t, i18n } = useTranslation();
  const [current, setCurrent] = useState<Locale>(() => {
    const lng = i18n.language;
    return lng === "en" ? "en" : "es";
  });
  const [themeMode, setThemeMode] = useTheme();

  const update = (loc: Locale) => {
    setCurrent(loc);
    setLocale(loc);
  };

  useEffect(() => {
    setCurrent(getLocale());
  }, [i18n.language]);

  return (
    <Card>
      <CardHeader
        title={t("settings.interface_title")}
        subtitle={t("settings.interface_subtitle")}
      />
      <CardBody className="space-y-4">
        <Field label={t("settings.interface_field")}>
          <div className="inline-flex rounded-lg border border-border bg-bg p-0.5">
            <LangPill
              active={current === "es"}
              onClick={() => update("es")}
            >
              {t("settings.language_es")}
            </LangPill>
            <LangPill
              active={current === "en"}
              onClick={() => update("en")}
            >
              {t("settings.language_en")}
            </LangPill>
          </div>
        </Field>
        <Field label={t("settings.theme_field")}>
          <div className="inline-flex rounded-lg border border-border bg-bg p-0.5">
            <LangPill
              active={themeMode === "system"}
              onClick={() => setThemeMode("system")}
            >
              {t("settings.theme_system")}
            </LangPill>
            <LangPill
              active={themeMode === "light"}
              onClick={() => setThemeMode("light")}
            >
              {t("settings.theme_light")}
            </LangPill>
            <LangPill
              active={themeMode === "dark"}
              onClick={() => setThemeMode("dark")}
            >
              {t("settings.theme_dark")}
            </LangPill>
          </div>
        </Field>
      </CardBody>
    </Card>
  );
}

function LangPill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-md px-3 py-1.5 text-[13px] font-medium transition",
        "focus:outline-none focus-visible:ring-conclave",
        active
          ? "bg-surface text-ink shadow-soft"
          : "text-ink-subtle hover:text-ink",
      )}
      aria-pressed={active}
    >
      {children}
    </button>
  );
}

// ===========================================================================
// Provider picker (empty state)
// ===========================================================================
function ProviderPicker({
  providers,
  busy,
  ollamaAvailable,
  onPick,
}: {
  providers: ProviderInfo[];
  busy: boolean;
  ollamaAvailable: boolean;
  onPick: (id: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="space-y-5 animate-in">
      {PICKER_GROUPS.map((group) => (
        <div key={group.titleKey}>
          <div className="mb-2 flex items-center gap-2 text-[11px] font-medium uppercase tracking-[0.08em] text-ink-faint">
            <span>{t(group.titleKey)}</span>
            <span className="h-px flex-1 bg-border-subtle" />
          </div>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {group.ids.map((id) => {
              const info = providers.find((p) => p.id === id);
              if (!info) return null;
              return (
                <PickerTile
                  key={id}
                  provider={info}
                  meta={metaFor(id)}
                  disabled={busy}
                  onPick={() => onPick(id)}
                />
              );
            })}
          </div>
        </div>
      ))}

      <OllamaNote available={ollamaAvailable} />
    </div>
  );
}

function PickerTile({
  provider,
  meta,
  disabled,
  onPick,
}: {
  provider: ProviderInfo;
  meta: ProviderMeta;
  disabled: boolean;
  onPick: () => void;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={onPick}
      disabled={disabled}
      aria-label={t("settings.picker_connect_aria", { name: meta.name })}
      className={cn(
        "group relative flex w-full items-start gap-3 rounded-lg border border-border bg-bg p-3.5 text-left transition-all",
        "hover:bg-surface-hover focus:outline-none focus-visible:ring-conclave",
        "disabled:cursor-not-allowed disabled:opacity-50",
        BRAND_HOVER[meta.brand],
      )}
    >
      <Monogram meta={meta} size={36} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <div className="truncate text-[13.5px] font-semibold text-ink">
            {meta.name}
          </div>
          {meta.recommended && (
            <span className="border border-ink px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-[0.08em] text-ink">
              {t("settings.picker_recommended")}
            </span>
          )}
        </div>
        <div className="mt-0.5 truncate text-[12px] text-ink-faint">
          {meta.tagline}
          {provider.hint && (
            <>
              {" · "}
              <span className="text-ink-subtle">{provider.hint}</span>
            </>
          )}
        </div>
      </div>
      <div className="shrink-0 self-center text-ink-faint transition-transform group-hover:translate-x-0.5 group-hover:text-ink-subtle">
        <Chevron />
      </div>
    </button>
  );
}

function OllamaNote({ available }: { available: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-lg border border-border-subtle bg-bg-subtle p-3.5 text-[12px] text-ink-subtle">
      <div className="mt-0.5 grid h-7 w-7 shrink-0 place-content-center rounded-md bg-slate-400/10 text-slate-300">
        <LocalIcon />
      </div>
      <div className="leading-relaxed">
        <span className="font-medium text-ink-dim">
          {t("settings.ollama_label")}
        </span>{" "}
        {available
          ? t("settings.ollama_running")
          : t("settings.ollama_pending")}
      </div>
    </div>
  );
}

// ===========================================================================
// Active provider (connected state)
// ===========================================================================
function ActiveProviderView({
  provider,
  busy,
  ollamaAvailable,
  onTest,
  onSwitch,
  onDisconnect,
}: {
  provider: ProviderInfo;
  busy: boolean;
  ollamaAvailable: boolean;
  onTest: () => void;
  onSwitch: () => void;
  onDisconnect: () => void;
}) {
  const { t } = useTranslation();
  const meta = metaFor(provider.id);
  return (
    <div className="space-y-4 animate-in">
      <div className="rounded-xl border border-border-strong bg-bg-elevated p-5 shadow-soft">
        <div className="flex items-start gap-4">
          <Monogram meta={meta} size={48} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <div className="text-[16px] font-semibold text-ink">
                {meta.name}
              </div>
              <span className="rounded bg-ok/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-ok">
                {t("settings.status_connected")}
              </span>
              <span
                className={cn(
                  "rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide",
                  provider.available
                    ? "bg-ok/10 text-ok"
                    : "bg-warn/15 text-warn",
                )}
              >
                {provider.available
                  ? t("settings.status_reachable")
                  : t("settings.status_unreachable")}
              </span>
            </div>
            <div className="mt-1 text-[13px] text-ink-subtle">
              {meta.tagline}
              {provider.hint && (
                <>
                  {" · "}
                  <span className="text-ink-dim">{provider.hint}</span>
                </>
              )}
            </div>
            <div className="mt-2 text-[12px] text-ink-faint">
              {t("settings.default_model")}{" "}
              <span className="font-mono text-ink-subtle">
                {provider.default_model}
              </span>
              {" · "}
              <span>{meta.authLabel}</span>
            </div>
          </div>
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-border-subtle pt-4">
          <Button size="sm" onClick={onTest} loading={busy}>
            {t("settings.action_test")}
          </Button>
          <Button size="sm" variant="ghost" onClick={onSwitch} disabled={busy}>
            {t("settings.action_switch")}
          </Button>
          <span className="flex-1" />
          <Button
            size="sm"
            variant="danger"
            onClick={onDisconnect}
            disabled={busy}
          >
            {t("settings.action_disconnect")}
          </Button>
        </div>
      </div>

      {ollamaAvailable && (
        <div className="flex items-center gap-2 px-1 text-[12px] text-ink-faint">
          <span className="h-1.5 w-1.5 rounded-full bg-slate-400" />
          <span>{t("settings.ollama_secondary")}</span>
        </div>
      )}
    </div>
  );
}

// ===========================================================================
// Connect flow (in-card overlay)
// ===========================================================================
function ConnectFlowView({
  flow,
  busy,
  onCancel,
  onSaveApiKey,
  onSubmitAnthropicCode,
  onUpdateAnthropicCode,
  onUpdateApiDraft,
}: {
  flow: ConnectFlow;
  busy: boolean;
  onCancel: () => void;
  onSaveApiKey: (draft: string) => void;
  onSubmitAnthropicCode: (code: string) => void;
  onUpdateAnthropicCode: (code: string) => void;
  onUpdateApiDraft: (draft: string) => void;
}) {
  const { t } = useTranslation();
  if (flow.kind === "idle") return null;

  const id =
    flow.kind === "api-key"
      ? flow.id
      : flow.kind === "oauth-anthropic"
        ? "anthropic-oauth"
        : "openai-oauth";
  const meta = metaFor(id);

  return (
    <div className="space-y-4 animate-in">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          aria-label={t("settings.back_aria")}
          className={cn(
            "grid h-8 w-8 place-content-center rounded-md border border-border-subtle text-ink-subtle transition",
            "hover:bg-surface hover:text-ink focus:outline-none focus-visible:ring-conclave",
            "disabled:cursor-not-allowed disabled:opacity-50",
          )}
        >
          <BackIcon />
        </button>
        <Monogram meta={meta} size={36} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-semibold text-ink">
            {t("settings.connect_title", { name: meta.name })}
          </div>
          <div className="truncate text-[12px] text-ink-faint">
            {meta.tagline} · {meta.authLabel}
          </div>
        </div>
      </div>

      <div className="rounded-lg border border-border-subtle bg-bg p-4">
        {flow.kind === "api-key" && (
          <ApiKeyForm
            meta={meta}
            draft={flow.draft}
            busy={busy}
            onChange={onUpdateApiDraft}
            onSave={() => onSaveApiKey(flow.draft)}
            onCancel={onCancel}
          />
        )}

        {flow.kind === "oauth-anthropic" && (
          <AnthropicOAuthFlow
            pasteInstructions={flow.pasteInstructions}
            code={flow.code}
            busy={busy}
            onChange={onUpdateAnthropicCode}
            onSubmit={() => onSubmitAnthropicCode(flow.code)}
            onCancel={onCancel}
          />
        )}

        {flow.kind === "oauth-openai" && <OpenAIOAuthFlow busy={busy} />}
      </div>
    </div>
  );
}

function ApiKeyForm({
  meta,
  draft,
  busy,
  onChange,
  onSave,
  onCancel,
}: {
  meta: ProviderMeta;
  draft: string;
  busy: boolean;
  onChange: (v: string) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSave();
      }}
      className="space-y-3"
    >
      <Field
        label={t("settings.apikey_field_label", { name: meta.name })}
        hint={t("settings.apikey_field_hint")}
      >
        <Input
          type="password"
          value={draft}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("settings.apikey_placeholder")}
          autoFocus
        />
      </Field>
      <div className="flex items-center gap-2">
        <Button
          type="submit"
          variant="primary"
          loading={busy}
          disabled={!draft.trim()}
        >
          {t("settings.connect_button")}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel} disabled={busy}>
          {t("settings.cancel")}
        </Button>
      </div>
    </form>
  );
}

function AnthropicOAuthFlow({
  pasteInstructions,
  code,
  busy,
  onChange,
  onSubmit,
  onCancel,
}: {
  pasteInstructions: string | null;
  code: string;
  busy: boolean;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
      className="space-y-3"
    >
      <div className="flex items-start gap-2 border border-border bg-surface p-3 text-[12px] leading-relaxed text-ink-dim">
        <span className="mt-0.5 text-ink-subtle">
          <InfoIcon />
        </span>
        <span>
          {pasteInstructions ?? t("settings.oauth_anthropic_fallback")}
        </span>
      </div>
      <Field label={t("settings.oauth_code_label")}>
        <Input
          value={code}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("settings.oauth_code_placeholder")}
          autoFocus
        />
      </Field>
      <div className="flex items-center gap-2">
        <Button
          type="submit"
          variant="primary"
          loading={busy}
          disabled={!code.trim()}
        >
          {t("settings.oauth_finish")}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel} disabled={busy}>
          {t("settings.cancel")}
        </Button>
      </div>
    </form>
  );
}

function OpenAIOAuthFlow({ busy }: { busy: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3 py-2">
      {busy && (
        <span className="inline-flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot" />
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot [animation-delay:120ms]" />
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot [animation-delay:240ms]" />
        </span>
      )}
      <div className="text-[13px] text-ink-dim">
        {t("settings.oauth_openai_waiting_title")}
        <div className="mt-1 text-[12px] text-ink-faint">
          {t("settings.oauth_openai_waiting_body")}
        </div>
      </div>
    </div>
  );
}

// ===========================================================================
// Migration dialog
// ===========================================================================
function MigrationDialog({
  candidates,
  busy,
  onKeep,
  onCancel,
}: {
  candidates: ProviderInfo[];
  busy: boolean;
  onKeep: (keepId: string) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string>(candidates[0]?.id ?? "");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onCancel]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="migration-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-bg/85 backdrop-blur"
    >
      <div className="animate-in mx-4 w-full max-w-md rounded-2xl border border-border bg-bg-elevated p-6 shadow-soft">
        <div className="mb-4">
          <h2
            id="migration-title"
            className="text-[15px] font-semibold text-ink"
          >
            {t("settings.migration_title")}
          </h2>
          <p className="mt-1 text-[13px] leading-relaxed text-ink-subtle">
            {t("settings.migration_body")}
          </p>
        </div>

        <div className="space-y-2">
          {candidates.map((p) => {
            const meta = metaFor(p.id);
            const isSelected = selected === p.id;
            return (
              <button
                key={p.id}
                type="button"
                onClick={() => setSelected(p.id)}
                className={cn(
                  "flex w-full items-center gap-3 rounded-lg border p-3 text-left transition",
                  "focus:outline-none focus-visible:ring-conclave",
                  isSelected
                    ? "border-ink bg-surface-active"
                    : "border-border bg-bg hover:bg-surface-hover",
                )}
              >
                <Monogram meta={meta} size={32} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px] font-medium text-ink">
                    {meta.name}
                  </div>
                  <div className="truncate text-[11.5px] text-ink-faint">
                    {meta.tagline} · {meta.authLabel}
                  </div>
                </div>
                <Radio selected={isSelected} />
              </button>
            );
          })}
        </div>

        <div className="mt-5 flex items-center justify-end gap-2">
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            {t("settings.migration_decide_later")}
          </Button>
          <Button
            variant="primary"
            onClick={() => onKeep(selected)}
            loading={busy}
            disabled={!selected}
          >
            {t("settings.migration_keep")}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ===========================================================================
// Small visual building blocks
// ===========================================================================
function Monogram({
  meta,
  size,
}: {
  meta: ProviderMeta;
  size: number;
}) {
  return (
    <div
      className={cn(
        "grid shrink-0 place-content-center rounded-lg ring-1 font-semibold",
        BRAND_TINT[meta.brand],
      )}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.42) }}
      aria-hidden
    >
      {meta.monogram}
    </div>
  );
}

function Radio({ selected }: { selected: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "grid h-4 w-4 shrink-0 place-content-center rounded-full border transition",
        selected ? "border-ink" : "border-border-strong",
      )}
    >
      {selected && <span className="h-1.5 w-1.5 rounded-full bg-ink" />}
    </span>
  );
}

function Chevron(): ReactNode {
  return (
    <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
      <path
        d="m9 6 6 6-6 6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function BackIcon(): ReactNode {
  return (
    <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
      <path
        d="m15 6-6 6 6 6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function InfoIcon(): ReactNode {
  return (
    <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
      <circle
        cx="12"
        cy="12"
        r="9"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path
        d="M12 11v5m0-8h.01"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function LocalIcon(): ReactNode {
  return (
    <svg viewBox="0 0 24 24" fill="none" className="h-3.5 w-3.5">
      <rect
        x="3.5"
        y="5"
        width="17"
        height="11"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path
        d="M8 20h8M12 16v4"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
