import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  IconChevronLeft,
  IconChevronRight,
  IconDeviceDesktop,
  IconInfoCircle,
} from "@tabler/icons-react";
import { open as openExternal } from "@tauri-apps/plugin-shell";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input } from "../components/Field";
import { ProviderStatusPill } from "../components/ProviderStatusPill";
import { cn } from "../lib/cn";
import { getLocale, setLocale, type Locale } from "../i18n";
import { useTheme } from "../lib/theme";
import {
  activeProvider,
  connectedSlotProviders,
  ipc,
  type CliDiagnostics,
  type DataBoundaryMode,
  type ProviderInfo,
} from "../lib/ipc";
import { isReady } from "../lib/providerStatus";
import {
  BRAND_HOVER,
  BRAND_TINT,
  CLI_INSTALL_URL,
  CLI_LOGIN_COMMAND,
  buildPickerGroups,
  isLocalCli,
  isSubscriptionOAuth,
  metaFor,
  shouldRecommendCli,
  type ProviderId,
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
  | { kind: "oauth-openai"; url: string }
  | { kind: "cli-setup"; id: "claude-cli" | "codex-cli" };

export function SettingsPage() {
  const { t } = useTranslation();

  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [privacyMode, setPrivacyMode] =
    useState<DataBoundaryMode>("deid_cloud");
  const [privacyBusy, setPrivacyBusy] = useState(false);

  const [flow, setFlow] = useState<ConnectFlow>({ kind: "idle" });
  const [migrationOpen, setMigrationOpen] = useState(false);
  const migrationDismissed = useRef(false);

  const active = useMemo(() => activeProvider(providers), [providers]);
  const ollama = useMemo(
    () => providers.find((p) => p.id === "ollama"),
    [providers],
  );
  // Surfaces Apple Intelligence when the host can plausibly run it.
  // The backend omits the entry on Intel Macs / macOS < 26, so
  // `appleIntel` ends up `undefined` for users with no path to using
  // it — every UI that consumes this value just skips the note.
  const appleIntel = useMemo(
    () => providers.find((p) => p.id === "apple-intelligence"),
    [providers],
  );

  const refresh = async (opts?: { force?: boolean }) => {
    setLoading(true);
    setError(null);
    try {
      const [list, privacy] = await Promise.all([
        // Force-refresh bypasses the 60s probe cache; we want this
        // when the user explicitly hits Reload or just finished a
        // test, so the pill reflects the freshest possible state.
        ipc.listProviders({ forceRefresh: opts?.force ?? false }),
        ipc.privacySettings().catch(() => null),
      ]);
      setProviders(list);
      if (privacy) {
        setPrivacyMode(privacy.default_data_boundary);
      }
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

  // Refresh when the window regains focus. Covers the common flow of
  // installing or logging into a CLI in a terminal while Conclave is
  // open in the background — without this the user has to hit Reload
  // to see the change. force=true bypasses the 60s probe cache AND
  // invalidates the binary detection cache on the backend.
  useEffect(() => {
    const onFocus = () => {
      void refresh({ force: true });
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
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

  // Kick off OpenAI's localhost-redirect flow as a fire-and-forget. The
  // backend spawns a background task that owns the :1455 listener; we just
  // transition the UI into "waiting" and let the polling effect detect
  // completion (or the back arrow trigger a cancel). We also stash the
  // authorize URL so the overlay can offer copy/open affordances — Brave
  // Shields and similar privacy tools regularly break the cross-subdomain
  // session OpenAI relies on, and the only reliable recovery is to open
  // the URL in a browser where ChatGPT is already signed in.
  const startOpenAIOAuth = async () => {
    setError(null);
    try {
      const r = await ipc.oauthOpenaiStart();
      setFlow({ kind: "oauth-openai", url: r.url });
    } catch (e) {
      setError(String(e));
      setFlow({ kind: "idle" });
    }
  };

  // While we're showing the OpenAI "waiting" overlay, poll list_providers
  // every 2 s so we notice when the background task persisted the token.
  // The effect's cleanup tells the backend to abort if the user backs out
  // of the flow without completing it — that's what releases :1455.
  useEffect(() => {
    if (flow.kind !== "oauth-openai") return;
    let cancelled = false;
    const interval = window.setInterval(async () => {
      try {
        const list = await ipc.listProviders();
        if (cancelled) return;
        setProviders(list);
        // The OAuth flow lands when the credentials file appears on
        // disk; the backend then probes immediately and the status
        // settles to `ready` (or `expired`/`unreachable` if the new
        // token isn't accepted, which we want to surface either way).
        const openaiOauth = list.find((p) => p.id === "openai-oauth");
        if (openaiOauth && openaiOauth.status !== "not_configured") {
          setFlow({ kind: "idle" });
        }
      } catch {
        // best-effort polling; ignore transient errors
      }
    }, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      void ipc.oauthOpenaiCancel().catch(() => {});
    };
  }, [flow.kind]);

  const pickProvider = async (id: string) => {
    const meta = metaFor(id);
    if (id === "anthropic-oauth") return startAnthropicOAuth();
    if (id === "openai-oauth") return startOpenAIOAuth();
    if (isLocalCli(id)) {
      // Two paths for CLI tiles depending on backend state:
      //
      //   • status === "ready"  → the binary is on $PATH and the CLI's
      //     own login is current. Picking the tile activates the
      //     provider (the backend call also clears the user-disabled
      //     flag that "Disconnect" sets in `conclave.toml`).
      //
      //   • anything else (NotInstalled / LoginRequired / Expired /
      //     NotConfigured) → open the in-Settings setup panel so the
      //     user gets actionable copy, an install button, a copy-able
      //     terminal command, and a Re-detect affordance — instead of
      //     a dead-end disabled tile.
      const info = providers.find((p) => p.id === id);
      // Always clear the user-disabled flag on a CLI tile click,
      // regardless of probe state. The user pressing the tile is an
      // explicit "I want this provider" signal — leaving the disabled
      // flag set would leave them stuck in `NotConfigured` (which
      // currently falls through to the LoginRequired UI in the setup
      // panel) even after they fix the underlying CLI login. We don't
      // surface errors here because the worst case is a no-op write.
      try {
        await ipc.setProviderKey(id, "");
      } catch {
        // best-effort; the setup panel still renders correctly
      }
      if (!info || !isReady(info)) {
        setFlow({ kind: "cli-setup", id: id as "claude-cli" | "codex-cli" });
        // Refresh providers in the background so the panel sees the
        // freshly-cleared flag on its next listProviders pull.
        void refresh();
        return;
      }
      setBusy(true);
      setError(null);
      try {
        await refresh();
      } finally {
        setBusy(false);
      }
      return;
    }
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
      // Regardless of outcome, force-refresh the provider list so
      // the status pill flips to match what the test just observed.
      // Without this the badges could stay green even after a failed
      // test — the exact mismatch this whole refactor fixes.
      await refresh({ force: true });
    }
  };

  const savePrivacyMode = async (mode: DataBoundaryMode) => {
    setPrivacyBusy(true);
    setError(null);
    try {
      const saved = await ipc.setPrivacySettings({
        default_data_boundary: mode,
      });
      setPrivacyMode(saved.default_data_boundary);
    } catch (e) {
      setError(String(e));
    } finally {
      setPrivacyBusy(false);
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
      <PrivacyCard
        mode={privacyMode}
        busy={privacyBusy}
        onChange={savePrivacyMode}
      />

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
              // Manual refresh: bypass the 60s probe cache so the
              // user actually gets a fresh upstream check.
              onClick={() => refresh({ force: true })}
              loading={loading}
            >
              {t("settings.refresh")}
            </Button>
          }
        />
        <CardBody className="space-y-4">
          {/* The banner now self-determines whether to render — it
              fires on `error` OR when the active provider's status
              is non-`ready`, so a revoked OAuth session surfaces
              without needing the user to click Test first. */}
          <ProviderErrorBanner
            error={error}
            activeProvider={active}
            onPickProvider={pickProvider}
            onRedetectCli={async () => {
              setBusy(true);
              try {
                await ipc.redetectCliBinaries();
                await refresh({ force: true });
              } finally {
                setBusy(false);
              }
            }}
          />


          {flow.kind !== "idle" ? (
            <ConnectFlowView
              flow={flow}
              busy={busy}
              providers={providers}
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
              onCliReady={(id) => {
                // Status flipped to ready while the panel was open
                // (e.g. user logged in via terminal and clicked
                // Re-detect). Close the panel and activate the
                // provider in the same beat.
                setFlow({ kind: "idle" });
                void (async () => {
                  setBusy(true);
                  try {
                    await ipc.setProviderKey(id, "");
                  } finally {
                    setBusy(false);
                    await refresh({ force: true });
                  }
                })();
              }}
              onRefreshProviders={() => refresh({ force: true })}
            />
          ) : active ? (
            <ActiveProviderView
              provider={active}
              busy={busy}
              ollamaAvailable={ollama ? isReady(ollama) : false}
              appleIntel={appleIntel}
              onTest={() => testActive(active.id)}
              onTestAppleIntel={() => testActive("apple-intelligence")}
              onSwitch={() => switchProvider(active)}
              onDisconnect={() => disconnect(active, { confirm: true })}
            />
          ) : (
            <ProviderPicker
              providers={providers}
              busy={busy}
              ollamaAvailable={ollama ? isReady(ollama) : false}
              appleIntel={appleIntel}
              onTestAppleIntel={() => testActive("apple-intelligence")}
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

function PrivacyCard({
  mode,
  busy,
  onChange,
}: {
  mode: DataBoundaryMode;
  busy: boolean;
  onChange: (mode: DataBoundaryMode) => void;
}) {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader
        title={t("settings.privacy_title")}
        subtitle={t("settings.privacy_subtitle")}
      />
      <CardBody>
        <Field label={t("settings.privacy_boundary_field")}>
          <select
            value={mode}
            disabled={busy}
            onChange={(e) => onChange(e.target.value as DataBoundaryMode)}
            className="w-full rounded-md border border-border-subtle bg-bg px-3 py-2 text-[13px] text-ink outline-none focus:ring-2 focus:ring-accent/40 disabled:opacity-60"
          >
            <option value="deid_cloud">
              {t("settings.privacy_boundary_deid_cloud")}
            </option>
            <option value="local_only">
              {t("settings.privacy_boundary_local_only")}
            </option>
            <option value="explicit_phi">
              {t("settings.privacy_boundary_explicit_phi")}
            </option>
          </select>
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
  appleIntel,
  onTestAppleIntel,
  onPick,
}: {
  providers: ProviderInfo[];
  busy: boolean;
  ollamaAvailable: boolean;
  appleIntel: ProviderInfo | undefined;
  onTestAppleIntel: () => void;
  onPick: (id: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="space-y-5 animate-in">
      {buildPickerGroups(providers).map((group) => {
        const isOAuthGroup = group.titleKey === "settings.picker_group_oauth";
        const isCliGroup = group.titleKey === "settings.picker_group_cli";
        const cliRecommended = isCliGroup && shouldRecommendCli(providers);
        return (
          <div key={group.titleKey}>
            <div className="flex items-center gap-2 text-[11px] font-medium uppercase tracking-[0.08em] text-ink-faint">
              <span>{t(group.titleKey)}</span>
              <span className="h-px flex-1 bg-border-subtle" />
            </div>
            {group.captionKey && (
              <p
                className={cn(
                  "mb-2 mt-1 text-[11.5px] leading-relaxed",
                  isOAuthGroup
                    ? "text-warn/90"
                    : isCliGroup && cliRecommended
                      ? "text-ok/90"
                      : "text-ink-faint",
                )}
              >
                {t(group.captionKey)}
              </p>
            )}
            <div
              className={cn(
                "grid grid-cols-1 gap-2 sm:grid-cols-2",
                !group.captionKey && "mt-2",
              )}
            >
              {group.ids.map((id) => {
                const info = providers.find((p) => p.id === id);
                if (!info) return null;
                // When CLI is the recommended group, override the
                // static meta.recommended flag so the chip pins to
                // the first signed-in CLI tile instead of to the API
                // key default.
                const meta = metaFor(id);
                const displayMeta: ProviderMeta =
                  isCliGroup && cliRecommended && id === "claude-cli"
                    ? { ...meta, recommended: true }
                    : isCliGroup && cliRecommended
                      ? { ...meta, recommended: false }
                      : cliRecommended && meta.recommended
                        ? { ...meta, recommended: false }
                        : meta;
                // CLI tiles used to lock when the binary wasn't ready
                // — a dead-end for the user. Now they always open the
                // setup panel (with diagnostics + install/login CTAs)
                // unless the whole picker is mid-action.
                return (
                  <PickerTile
                    key={id}
                    provider={info}
                    meta={displayMeta}
                    disabled={busy}
                    onPick={() => onPick(id)}
                  />
                );
              })}
            </div>
          </div>
        );
      })}

      <OllamaNote available={ollamaAvailable} />
      <AppleIntelligenceNote
        info={appleIntel}
        busy={busy}
        onTest={onTestAppleIntel}
      />
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
              <span className="text-ink-subtle">
                {hintLabel(t, provider.id, provider.hint)}
              </span>
            </>
          )}
        </div>
      </div>
      <div className="shrink-0 self-center text-ink-faint transition-transform group-hover:translate-x-0.5 group-hover:text-ink-subtle">
        <IconChevronRight size={16} stroke={1.5} />
      </div>
    </button>
  );
}

function OllamaNote({ available }: { available: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-lg border border-border-subtle bg-bg-subtle p-3.5 text-[12px] text-ink-subtle">
      <div className="mt-0.5 grid h-7 w-7 shrink-0 place-content-center rounded-md bg-slate-400/10 text-slate-300">
        <IconDeviceDesktop size={14} stroke={1.5} />
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

// Apple Intelligence note. Mirrors the Ollama pattern (informational
// tile, not a picker tile, because there's nothing to "connect" to)
// but with a small Test button when the model is ready — there's no
// active-provider flow that would surface this provider otherwise,
// since it's barred from the clinical pickers.
//
// Renders nothing when `info` is undefined (hard-unavailable hosts).
function AppleIntelligenceNote({
  info,
  busy,
  onTest,
}: {
  info: ProviderInfo | undefined;
  busy: boolean;
  onTest: () => void;
}) {
  const { t } = useTranslation();
  if (!info) return null;
  // `hint` carries the stable Availability tag ("not_enabled",
  // "downloading", etc.) when the provider isn't fully ready, so the
  // copy can be precise about what the user can do.
  const stateKey = isReady(info)
    ? "settings.apple_intel_ready"
    : info.hint === "not_enabled"
      ? "settings.apple_intel_unavailable_not_enabled"
      : info.hint === "downloading"
        ? "settings.apple_intel_unavailable_downloading"
        : "settings.apple_intel_unavailable_other";
  return (
    <div className="flex items-start gap-3 rounded-lg border border-border-subtle bg-bg-subtle p-3.5 text-[12px] text-ink-subtle">
      <div className="mt-0.5 grid h-7 w-7 shrink-0 place-content-center rounded-md bg-sky-400/10 text-sky-300">
        <IconDeviceDesktop size={14} stroke={1.5} />
      </div>
      <div className="min-w-0 flex-1 leading-relaxed">
        <span className="font-medium text-ink-dim">
          {t("settings.apple_intel_label")}
        </span>{" "}
        {t(stateKey)}
      </div>
      {isReady(info) && (
        <Button
          size="sm"
          variant="ghost"
          onClick={onTest}
          loading={busy}
          className="shrink-0"
        >
          {t("settings.action_test")}
        </Button>
      )}
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
  appleIntel,
  onTest,
  onTestAppleIntel,
  onSwitch,
  onDisconnect,
}: {
  provider: ProviderInfo;
  busy: boolean;
  ollamaAvailable: boolean;
  appleIntel: ProviderInfo | undefined;
  onTest: () => void;
  onTestAppleIntel: () => void;
  onSwitch: () => void;
  onDisconnect: () => void;
}) {
  const { t } = useTranslation();
  const meta = metaFor(provider.id);
  const unofficial = isSubscriptionOAuth(provider.id);
  const officialCli = isLocalCli(provider.id);
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
              {/* Single status pill — replaces the old
                  "CONECTADO" + "ALCANZABLE" pair that could disagree
                  with the error banner below. One source of truth,
                  refreshed after every test and OAuth callback. */}
              <ProviderStatusPill status={provider.status} />
              {unofficial && (
                <span
                  className="rounded bg-warn/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-warn"
                  title={t("settings.oauth_active_caption")}
                >
                  {t("settings.oauth_unofficial_badge")}
                </span>
              )}
              {officialCli && (
                <span
                  className="rounded bg-ok/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-ok"
                  title={t("settings.cli_active_caption")}
                >
                  {t("settings.cli_official_badge")}
                </span>
              )}
            </div>
            <div className="mt-1 text-[13px] text-ink-subtle">
              {meta.tagline}
              {provider.hint && (
                <>
                  {" · "}
                  <span className="text-ink-dim">
                    {hintLabel(t, provider.id, provider.hint)}
                  </span>
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
            {unofficial && (
              <p className="mt-2 text-[11.5px] leading-relaxed text-warn/90">
                {t("settings.oauth_active_caption")}
              </p>
            )}
            {officialCli && (
              <p className="mt-2 text-[11.5px] leading-relaxed text-ok/90">
                {t("settings.cli_active_caption")}
              </p>
            )}
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
      <AppleIntelligenceNote
        info={appleIntel}
        busy={busy}
        onTest={onTestAppleIntel}
      />
    </div>
  );
}

// ===========================================================================
// Connect flow (in-card overlay)
// ===========================================================================
function ConnectFlowView({
  flow,
  busy,
  providers,
  onCancel,
  onSaveApiKey,
  onSubmitAnthropicCode,
  onUpdateAnthropicCode,
  onUpdateApiDraft,
  onCliReady,
  onRefreshProviders,
}: {
  flow: ConnectFlow;
  busy: boolean;
  providers: ProviderInfo[];
  onCancel: () => void;
  onSaveApiKey: (draft: string) => void;
  onSubmitAnthropicCode: (code: string) => void;
  onUpdateAnthropicCode: (code: string) => void;
  onUpdateApiDraft: (draft: string) => void;
  onCliReady: (id: "claude-cli" | "codex-cli") => void;
  onRefreshProviders: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  if (flow.kind === "idle") return null;

  const id =
    flow.kind === "api-key"
      ? flow.id
      : flow.kind === "oauth-anthropic"
        ? "anthropic-oauth"
        : flow.kind === "oauth-openai"
          ? "openai-oauth"
          : flow.id;
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
          <IconChevronLeft size={16} stroke={1.5} />
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

        {flow.kind === "oauth-openai" && <OpenAIOAuthFlow url={flow.url} />}

        {flow.kind === "cli-setup" && (
          <CliSetupPanel
            id={flow.id}
            provider={providers.find((p) => p.id === flow.id) ?? null}
            onReady={() => onCliReady(flow.id)}
            onRefreshProviders={onRefreshProviders}
          />
        )}
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
      <OAuthDisclaimerBanner />
      <div className="flex items-start gap-2 border border-border bg-surface p-3 text-[12px] leading-relaxed text-ink-dim">
        <span className="mt-0.5 text-ink-subtle">
          <IconInfoCircle size={16} stroke={1.5} />
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

function OpenAIOAuthFlow({ url }: { url: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copyUrl = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // clipboard API may be unavailable in some webviews — ignore
    }
  };

  const openUrl = () => {
    window.open(url, "_blank", "noopener,noreferrer");
  };

  return (
    <div className="space-y-3">
      <OAuthDisclaimerBanner />
      <div className="flex items-center gap-3">
        <span className="inline-flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot" />
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot [animation-delay:120ms]" />
          <span className="h-1.5 w-1.5 rounded-full bg-accent animate-pulseDot [animation-delay:240ms]" />
        </span>
        <div className="text-[13px] text-ink-dim">
          {t("settings.oauth_openai_waiting_title")}
        </div>
      </div>

      <p className="text-[12px] leading-relaxed text-ink-faint">
        {t("settings.oauth_openai_waiting_body")}
      </p>

      <div className="rounded-md border border-border-subtle bg-bg-subtle p-2.5">
        <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-ink-faint">
          {t("settings.oauth_openai_url_label")}
        </div>
        <div className="flex items-center gap-2">
          <code
            className="block flex-1 truncate rounded bg-bg px-2 py-1.5 font-mono text-[11px] text-ink-subtle"
            title={url}
          >
            {url}
          </code>
          <button
            type="button"
            onClick={copyUrl}
            className={cn(
              "shrink-0 rounded-md border px-2.5 py-1.5 text-[12px] font-medium transition no-drag",
              "focus:outline-none focus-visible:ring-conclave",
              copied
                ? "border-ok/40 bg-ok/10 text-ok"
                : "border-border bg-surface text-ink-dim hover:bg-surface-hover hover:text-ink",
            )}
          >
            {copied
              ? t("settings.oauth_openai_url_copied")
              : t("settings.oauth_openai_url_copy")}
          </button>
        </div>
        <button
          type="button"
          onClick={openUrl}
          className="mt-2 text-[11.5px] text-accent transition no-drag hover:text-accent-strong focus:outline-none focus-visible:underline"
        >
          {t("settings.oauth_openai_url_open")}
        </button>
      </div>

      <p className="text-[11.5px] leading-relaxed text-ink-faint">
        {t("settings.oauth_openai_trouble_hint")}
      </p>
    </div>
  );
}

// ===========================================================================
// CLI setup panel — drives the in-Settings recovery flow for the two
// local-CLI providers (`claude-cli`, `codex-cli`).
//
// Surfaces three variants driven by the live `ProviderInfo.status`:
//
//   • `not_installed` — binary missing from `$PATH`. Shows the install
//     URL with an Open button and a collapsible "PATH seen by Conclave"
//     debug block so users with custom install dirs can diagnose.
//   • `login_required` / `expired` — binary found but no session. Shows
//     the detected path and a copy-able terminal command.
//   • `ready` — auto-activates: fires `onReady` which closes the panel
//     and writes the provider to the slot.
//
// The Re-detect button invalidates the backend's binary cache and
// refreshes the provider list. Combined with the parent's `focus`
// listener it covers the common flow "install/login in another window,
// come back to Conclave".
// ===========================================================================
function CliSetupPanel({
  id,
  provider,
  onReady,
  onRefreshProviders,
}: {
  id: "claude-cli" | "codex-cli";
  provider: ProviderInfo | null;
  onReady: () => void;
  onRefreshProviders: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const [diagnostics, setDiagnostics] = useState<CliDiagnostics | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [marking, setMarking] = useState(false);
  const [copied, setCopied] = useState(false);
  const [showPath, setShowPath] = useState(false);
  const [showProbe, setShowProbe] = useState(false);
  // Once we've called onReady (status flipped to ready) we suppress
  // future fires from re-renders that arrive before the parent
  // re-renders us with `flow.kind === "idle"`.
  const readyFiredRef = useRef(false);

  const meta = metaFor(id);
  const binary = id === "claude-cli" ? "claude" : "codex";
  const loginCommand = CLI_LOGIN_COMMAND[id];
  const installUrl = CLI_INSTALL_URL[id];

  // Fetch diagnostics on mount and whenever the parent-reported status
  // changes (after a Re-detect → refresh round-trip, for example).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const d = await ipc.cliDiagnostics(id);
        if (!cancelled) setDiagnostics(d);
      } catch {
        // best-effort; the panel still renders useful copy without it
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, provider?.status]);

  // Auto-activate when status reaches ready. We watch the parent-passed
  // `provider.status` rather than diagnostics.status so we react to the
  // exact same payload the rest of Settings is using.
  useEffect(() => {
    if (provider?.status === "ready" && !readyFiredRef.current) {
      readyFiredRef.current = true;
      onReady();
    }
  }, [provider?.status, onReady]);

  const redetect = async () => {
    setDetecting(true);
    try {
      await ipc.redetectCliBinaries();
      // Pull fresh provider list AND fresh diagnostics so the panel
      // updates in lock-step. listProviders with force=true also calls
      // refresh_binary_cache on the backend (belt-and-braces).
      await onRefreshProviders();
      const d = await ipc.cliDiagnostics(id);
      setDiagnostics(d);
    } finally {
      setDetecting(false);
    }
  };

  // Manual override toggle: persists `cli_local_overrides[id]` in
  // conclave.toml. We refresh the parent provider list afterwards so
  // the chip animates from "Inicia sesión en el CLI" to "Listo"
  // immediately. Clearing the override (true → false) is the same
  // command with `value: false`.
  const setOverride = async (value: boolean) => {
    setMarking(true);
    try {
      await ipc.setCliLoginOverride(id, value);
      await onRefreshProviders();
      const d = await ipc.cliDiagnostics(id);
      setDiagnostics(d);
    } finally {
      setMarking(false);
    }
  };

  const copyCommand = async () => {
    try {
      await navigator.clipboard.writeText(loginCommand);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // clipboard unavailable; the command is visible verbatim already
    }
  };

  const openInstallPage = async () => {
    try {
      await openExternal(installUrl);
    } catch {
      // shell plugin not available (unlikely); fall back to webview
      window.open(installUrl, "_blank", "noopener,noreferrer");
    }
  };

  const status = provider?.status ?? diagnostics?.status ?? "not_installed";
  const isNotInstalled = status === "not_installed";
  const isExpired = status === "expired";
  const binaryPath = diagnostics?.binary_path;

  // While we're transitioning to the active card, show a transient
  // "Activating…" state instead of the variant copy. Avoids a 1-frame
  // flash of "you're logged in!" right before the parent unmounts us.
  if (status === "ready") {
    return (
      <div className="space-y-3 animate-in">
        <div className="flex items-center gap-2 text-[13px] text-ink-dim">
          <span className="inline-flex items-center gap-1.5">
            <span className="h-1.5 w-1.5 rounded-full bg-ok animate-pulseDot" />
            <span className="h-1.5 w-1.5 rounded-full bg-ok animate-pulseDot [animation-delay:120ms]" />
            <span className="h-1.5 w-1.5 rounded-full bg-ok animate-pulseDot [animation-delay:240ms]" />
          </span>
          {t("settings.cli_setup_ready_redirecting")}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4 animate-in">
      {/* Headline + body copy per variant. We keep the wording terse
          and action-oriented; the actual CTAs live below. */}
      <div className="space-y-1.5">
        <h3 className="text-[14px] font-semibold text-ink">
          {isNotInstalled
            ? t("settings.cli_setup_not_installed_title", { binary: `\`${binary}\`` })
            : isExpired
              ? t("settings.cli_setup_expired_title", { name: meta.name })
              : t("settings.cli_setup_login_required_title", { name: meta.name })}
        </h3>
        <p className="text-[12.5px] leading-relaxed text-ink-subtle">
          {isNotInstalled
            ? t("settings.cli_setup_not_installed_body")
            : isExpired
              ? t("settings.cli_setup_expired_body")
              : t("settings.cli_setup_login_required_body")}
        </p>
        {binaryPath && !isNotInstalled && (
          <p className="text-[11.5px] text-ink-faint">
            <span className="text-ink-subtle">{t("settings.cli_setup_detected_at")}</span>{" "}
            <code className="font-mono text-ink-dim">{binaryPath}</code>
          </p>
        )}
      </div>

      {/* Variant-specific main CTA */}
      {isNotInstalled ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="primary" onClick={openInstallPage}>
            {t("settings.cli_setup_open_install_page")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={redetect}
            loading={detecting}
          >
            {detecting
              ? t("settings.cli_setup_redetecting")
              : t("settings.cli_setup_redetect")}
          </Button>
        </div>
      ) : (
        <div
          className={cn(
            "rounded-md border bg-bg-subtle p-2.5",
            isExpired ? "border-warn/40" : "border-border-subtle",
          )}
        >
          <div className="flex items-center gap-2">
            <code className="block flex-1 truncate rounded bg-bg px-2 py-1.5 font-mono text-[11.5px] text-ink-subtle">
              {loginCommand}
            </code>
            <button
              type="button"
              onClick={copyCommand}
              className={cn(
                "shrink-0 rounded-md border px-2.5 py-1.5 text-[12px] font-medium transition no-drag",
                "focus:outline-none focus-visible:ring-conclave",
                copied
                  ? "border-ok/40 bg-ok/10 text-ok"
                  : "border-border bg-surface text-ink-dim hover:bg-surface-hover hover:text-ink",
              )}
            >
              {copied
                ? t("settings.cli_setup_copy_command_copied")
                : t("settings.cli_setup_copy_command")}
            </button>
            <Button
              size="sm"
              variant="ghost"
              onClick={redetect}
              loading={detecting}
            >
              {detecting
                ? t("settings.cli_setup_redetecting")
                : t("settings.cli_setup_redetect")}
            </Button>
          </div>
        </div>
      )}

      {/* Collapsible "what PATH does Conclave see" diagnostic. Only
          relevant when the binary is missing — for the login flow the
          PATH is already proven correct by the detected path above. */}
      {isNotInstalled && diagnostics && (
        <div className="rounded-md border border-border-subtle bg-bg-subtle p-2.5">
          <button
            type="button"
            onClick={() => setShowPath((v) => !v)}
            className="text-[11.5px] font-medium text-ink-subtle transition no-drag hover:text-ink focus:outline-none focus-visible:underline"
          >
            {showPath
              ? t("settings.cli_setup_path_seen_hide")
              : t("settings.cli_setup_path_seen_show")}
            <span className="ml-1 text-ink-faint">
              ({t("settings.cli_setup_path_seen_label")})
            </span>
          </button>
          {showPath && (
            <>
              <pre className="mt-2 max-h-32 overflow-auto rounded bg-bg px-2 py-1.5 font-mono text-[10.5px] leading-relaxed text-ink-subtle">
                {diagnostics.path_var.split(":").join("\n")}
              </pre>
              <p className="mt-1.5 text-[11px] leading-relaxed text-ink-faint">
                {t("settings.cli_setup_path_seen_help")}
              </p>
            </>
          )}
        </div>
      )}

      {/* Manual override safety net — only shown when the binary IS on
          PATH but auto-detection couldn't confirm login. Honest copy:
          the user takes responsibility for declaring their state, and
          Conclave trusts it until the binary disappears. */}
      {!isNotInstalled && diagnostics && (
        <div className="rounded-md border border-border-subtle bg-bg-subtle p-2.5">
          {diagnostics.user_marked_ready ? (
            <div className="flex items-center justify-between gap-3">
              <p className="text-[11.5px] leading-relaxed text-ink-subtle">
                {t("settings.cli_setup_marked_ready_caption", {
                  binary: `\`${binary}\``,
                })}
              </p>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setOverride(false)}
                loading={marking}
              >
                {t("settings.cli_setup_unmark_connected")}
              </Button>
            </div>
          ) : (
            <div className="flex items-center justify-between gap-3">
              <p className="text-[11.5px] leading-relaxed text-ink-subtle">
                {t("settings.cli_setup_mark_connected_caption", {
                  binary: `\`${binary}\``,
                })}
              </p>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => setOverride(true)}
                loading={marking}
              >
                {t("settings.cli_setup_mark_connected")}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Technical diagnostics — what the probe actually returned. We
          surface command, exit code, duration, stderr excerpt, fallback
          used, env keys seen, and binary mtime/size so the next "no
          detecta" report ships with one screenshot's worth of detail.
          Collapsed by default so the panel stays calm. */}
      {!isNotInstalled && diagnostics && (
        <div className="rounded-md border border-border-subtle bg-bg-subtle p-2.5">
          <button
            type="button"
            onClick={() => setShowProbe((v) => !v)}
            className="text-[11.5px] font-medium text-ink-subtle transition no-drag hover:text-ink focus:outline-none focus-visible:underline"
          >
            {showProbe
              ? t("settings.cli_setup_probe_hide")
              : t("settings.cli_setup_probe_show")}
            <span className="ml-1 text-ink-faint">
              ({t("settings.cli_setup_probe_label")})
            </span>
          </button>
          {showProbe && <ProbeDetailsBlock diagnostics={diagnostics} />}
        </div>
      )}
    </div>
  );
}

/**
 * Render the raw login-probe payload for the CLI setup panel. Pure
 * read-out — no actions live here; the "Marcar como conectado" button
 * is rendered separately above so it's reachable without scrolling
 * past the debug block.
 */
function ProbeDetailsBlock({ diagnostics }: { diagnostics: CliDiagnostics }) {
  const { t } = useTranslation();
  const { probe } = diagnostics;
  const row = (label: string, value: ReactNode) => (
    <div className="flex gap-2 py-0.5">
      <span className="shrink-0 text-ink-faint">{label}</span>
      <span className="min-w-0 flex-1 break-words font-mono text-ink-subtle">
        {value}
      </span>
    </div>
  );
  return (
    <div className="mt-2 space-y-0 rounded bg-bg px-2 py-1.5 font-mono text-[10.5px] leading-relaxed">
      {probe.command && row(t("settings.cli_setup_probe_command"), probe.command)}
      {row(
        t("settings.cli_setup_probe_exit_code"),
        probe.exit_code === null ? "—" : String(probe.exit_code),
      )}
      {row(
        t("settings.cli_setup_probe_duration"),
        probe.duration_ms === null ? "—" : `${probe.duration_ms} ms`,
      )}
      {probe.timed_out && row(t("settings.cli_setup_probe_timeout"), "true")}
      {row(
        t("settings.cli_setup_probe_logged_in"),
        probe.logged_in ? "true" : "false",
      )}
      {probe.fallback_used &&
        row(t("settings.cli_setup_probe_fallback"), probe.fallback_used)}
      {probe.env_keys_seen.length > 0 &&
        row(
          t("settings.cli_setup_probe_env"),
          probe.env_keys_seen.join(", "),
        )}
      {probe.binary_size !== null &&
        row(
          t("settings.cli_setup_probe_binary"),
          `${probe.binary_size} bytes` +
            (probe.binary_mtime !== null
              ? ` · mtime ${new Date(probe.binary_mtime * 1000).toISOString()}`
              : ""),
        )}
      {probe.stderr_excerpt &&
        row(
          t("settings.cli_setup_probe_stderr"),
          <span className="whitespace-pre-wrap">{probe.stderr_excerpt}</span>,
        )}
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
      className="fixed inset-0 z-50 flex items-center justify-center bg-bg/85 px-4 pb-4 pt-14 backdrop-blur"
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

// Warning banner shown at the top of every OAuth flow (Anthropic /
// OpenAI subscription sign-in). The OAuth path reuses the official CLI
// client_id and is not a vendor-supported integration — access can be
// revoked at any time. We surface that upfront so the user accepts the
// trade-off knowingly.
function OAuthDisclaimerBanner() {
  const { t } = useTranslation();
  return (
    <div
      role="note"
      className="flex items-start gap-2 rounded-lg border border-warn/40 bg-warn/10 p-3 text-[12px] leading-relaxed text-warn"
    >
      <span className="mt-0.5 shrink-0">
        <IconInfoCircle size={16} stroke={1.5} />
      </span>
      <span className="text-warn/90">{t("settings.oauth_flow_disclaimer")}</span>
    </div>
  );
}

/**
 * The OAuth subscription path has a vendor-recommended API-key
 * fallback: the same vendor publishes both. When the OAuth session is
 * revoked, we surface a CTA that jumps straight into the picker for
 * the API-key counterpart so the user has a one-click stable path.
 */
function apiKeyAlternativeFor(id: string): ProviderId | null {
  if (id === "openai-oauth") return "openai";
  if (id === "anthropic-oauth") return "anthropic";
  return null;
}

/**
 * Banner shown when an active CLI provider's session has expired.
 * Carries both a copy-to-clipboard affordance for the exact terminal
 * command the user needs to run AND a Re-detect button that bypasses
 * the backend's probe cache. The OAuth side has a parallel "Switch to
 * API key" CTA — the CLI side gets parity here.
 */
function CliExpiredBanner({
  name,
  cliCommand,
  error,
  onRedetect,
}: {
  name: string;
  cliCommand: string;
  error: string | null;
  onRedetect?: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [redetecting, setRedetecting] = useState(false);

  const copy = async () => {
    if (!cliCommand) return;
    try {
      await navigator.clipboard.writeText(cliCommand);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      // clipboard unavailable — the command is visible verbatim
    }
  };

  const redetect = async () => {
    if (!onRedetect) return;
    setRedetecting(true);
    try {
      await onRedetect();
    } finally {
      setRedetecting(false);
    }
  };

  return (
    <div
      role="alert"
      className="animate-in space-y-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
    >
      <p>
        {t("settings.cli_error_not_logged_in", {
          name,
          command: cliCommand,
        })}
      </p>
      {error && (
        <p className="font-mono text-[11px] text-danger/80">{error}</p>
      )}
      <div className="flex flex-wrap items-center gap-2 pt-0.5">
        {cliCommand && (
          <button
            type="button"
            onClick={copy}
            className={cn(
              "rounded-md border px-2.5 py-1.5 text-[12px] font-medium transition no-drag",
              "focus:outline-none focus-visible:ring-conclave",
              copied
                ? "border-ok/40 bg-ok/10 text-ok"
                : "border-danger/40 bg-bg text-danger hover:bg-danger/15",
            )}
          >
            {copied
              ? t("settings.cli_setup_copy_command_copied")
              : t("settings.cli_error_copy_command")}
          </button>
        )}
        {onRedetect && (
          <Button
            size="sm"
            variant="ghost"
            onClick={redetect}
            loading={redetecting}
          >
            {t("settings.cli_error_redetect")}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * Status-driven banner shown beneath the active-provider card.
 * Replaces the old substring-sniffing version which matched
 * "authentication failed" against an opaque error string from
 * `ProviderError::Display` — that approach broke when the wire shape
 * changed and silently kept the banner green for non-auth failures.
 *
 * The new version is driven by the typed `ProviderInfo.status` from
 * the backend probe + the (optional) `error` from a recent test/action.
 * It returns `null` when there's nothing to surface, so it can be
 * mounted unconditionally without leaving an empty card.
 */
function ProviderErrorBanner({
  error,
  activeProvider,
  onPickProvider,
  onRedetectCli,
}: {
  error: string | null;
  activeProvider: ProviderInfo | null;
  onPickProvider?: (id: string) => void;
  onRedetectCli?: () => Promise<void> | void;
}) {
  const { t } = useTranslation();

  // Nothing to say: no active provider AND no error.
  if (!activeProvider && !error) return null;

  // No active provider but we have an error (rare — usually means
  // listProviders itself failed). Surface the raw text.
  if (!activeProvider) {
    return (
      <div
        role="alert"
        className="animate-in rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
      >
        {error}
      </div>
    );
  }

  const name = metaFor(activeProvider.id).name;
  const status = activeProvider.status;
  const isOAuth = isSubscriptionOAuth(activeProvider.id);
  const isCli = isLocalCli(activeProvider.id);
  const apiKeyAlt = apiKeyAlternativeFor(activeProvider.id);

  // Status === "expired": OAuth revoked or CLI logged out. Show the
  // path-specific recovery guidance + the recommended fallback CTA.
  if (status === "expired" && isOAuth) {
    return (
      <div
        role="alert"
        className="animate-in space-y-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
      >
        <p>{t("settings.oauth_error_revoked", { name })}</p>
        {error && (
          <p className="font-mono text-[11px] text-danger/80">{error}</p>
        )}
        {apiKeyAlt && onPickProvider && (
          <div className="pt-1">
            <Button
              size="sm"
              variant="primary"
              onClick={() => onPickProvider(apiKeyAlt)}
            >
              {t("settings.switch_to_api_key_cta")}
            </Button>
          </div>
        )}
      </div>
    );
  }
  if (status === "expired" && isCli) {
    const cliId = activeProvider.id as "claude-cli" | "codex-cli";
    const cliCommand = CLI_LOGIN_COMMAND[cliId] ?? "";
    return (
      <CliExpiredBanner
        name={name}
        cliCommand={cliCommand}
        error={error}
        onRedetect={onRedetectCli}
      />
    );
  }
  // Status === "unreachable": transient transport/timeout. Amber
  // tone (not danger) — the credential is still good, just the
  // upstream isn't answering right now.
  if (status === "unreachable") {
    return (
      <div
        role="alert"
        className="animate-in space-y-1 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[13px] text-warn"
      >
        <p>{t("settings.status_unreachable_caption", { name })}</p>
        {error && (
          <p className="font-mono text-[11px] text-warn/80">{error}</p>
        )}
      </div>
    );
  }

  // Status is "ready" (or anything else we don't surface here) — fall
  // back to plain error rendering if the user just hit something.
  if (!error) return null;
  return (
    <div
      role="alert"
      className="animate-in rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
    >
      {error}
    </div>
  );
}

/**
 * Resolve the human-readable label for a `ProviderInfo.hint` value.
 *
 * Most hints are passed through verbatim (subscription type for the
 * OAuth tiles, vendor-side messages for API keys). CLI providers
 * emit stable tags (`not_installed`, `login_required`) which we map
 * to provider-specific i18n strings so the user sees an actionable
 * instruction like "Run `claude auth login` in a terminal".
 */
function hintLabel(
  t: (key: string) => string,
  providerId: string,
  hint: string,
): string {
  if (!isLocalCli(providerId)) return hint;
  if (hint === "not_installed") {
    return t(
      providerId === "claude-cli"
        ? "settings.cli_hint_not_installed_claude"
        : "settings.cli_hint_not_installed_codex",
    );
  }
  if (hint === "login_required") {
    return t(
      providerId === "claude-cli"
        ? "settings.cli_hint_login_required_claude"
        : "settings.cli_hint_login_required_codex",
    );
  }
  return hint;
}

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
