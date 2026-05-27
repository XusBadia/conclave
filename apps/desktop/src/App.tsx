import { useEffect, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { Onboarding } from "./components/Onboarding";
import { ProviderStatusPill } from "./components/ProviderStatusPill";
import { Sidebar, type Section } from "./components/Sidebar";
import { CasesPage } from "./routes/Cases";
import { KnowledgePage } from "./routes/Knowledge";
import { SettingsPage } from "./routes/Settings";
import { WorkspacesPage } from "./routes/Workspaces";
import { activeProvider, ipc, type ProviderInfo, type Workspace } from "./lib/ipc";
import { metaFor } from "./lib/providers";

// Belt-and-suspenders drag handler. data-tauri-drag-region requires the
// core:window:allow-start-dragging permission (granted via capabilities);
// this explicit call covers any edge case where the attribute binding
// doesn't fire (e.g. event capture, future Tauri changes).
const startWindowDrag = async (e: ReactPointerEvent<HTMLElement>) => {
  const target = e.target as HTMLElement;
  if (target.closest('[data-tauri-drag-region="false"]')) return;
  if (target.closest("button, a, input, textarea, select")) return;
  try {
    await getCurrentWindow().startDragging();
  } catch {
    /* not running under Tauri (Vite preview) — ignore */
  }
};

export function App() {
  const { t } = useTranslation();
  const [section, setSection] = useState<Section>("workspaces");
  const [active, setActive] = useState<Workspace | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[] | null>(null);
  const [bootstrap, setBootstrap] = useState<{
    accepted: boolean;
    disclaimerEn: string;
    disclaimerEs: string;
  } | null>(null);

  useEffect(() => {
    (async () => {
      const status = await ipc.onboardingStatus();
      setBootstrap({
        accepted: status.accepted,
        disclaimerEn: status.disclaimer_en,
        disclaimerEs: status.disclaimer_es,
      });
      try {
        setActive(await ipc.activeWorkspace());
      } catch {
        setActive(null);
      }
    })();
  }, []);

  // App-wide provider snapshot for the title-bar pill and the
  // sidebar workspace card. Refreshed on mount, on a slow 60s
  // interval while the window is focused, and immediately when the
  // window regains focus (the user may have reconnected the LLM in
  // a CLI or browser tab between blurs). The 60s tick aligns with
  // the backend's probe cache TTL so we never trigger a real probe
  // more than once per minute.
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const list = await ipc.listProviders();
        if (!cancelled) setProviders(list);
      } catch {
        /* best-effort polling */
      }
    };
    void tick();
    const interval = window.setInterval(() => {
      if (document.visibilityState === "visible") void tick();
    }, 60_000);
    const onFocus = () => void tick();
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, []);

  const activeProviderInfo = providers ? activeProvider(providers) : null;

  if (!bootstrap) {
    return (
      <div className="grid h-full w-full place-content-center text-ink-faint">
        {t("common.loading")}
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
      {!bootstrap.accepted && (
        <Onboarding
          disclaimerEn={bootstrap.disclaimerEn}
          disclaimerEs={bootstrap.disclaimerEs}
          onAccepted={() => setBootstrap({ ...bootstrap, accepted: true })}
        />
      )}

      {/* macOS overlay title bar — spans the full window so the user can
          drag from anywhere. Uses Tauri 2's data-tauri-drag-region; any
          interactive descendant must set data-tauri-drag-region="false". */}
      <header
        data-tauri-drag-region
        onPointerDown={startWindowDrag}
        className="titlebar titlebar-pad-mac flex items-center gap-4 pr-5 text-[11px] text-ink-faint"
      >
        <div
          data-tauri-drag-region
          className="font-mono text-[11px] uppercase tracking-[0.16em] text-ink"
        >
          {t("app.brand")}
        </div>
        <span
          data-tauri-drag-region
          aria-hidden
          className="h-3 w-px bg-border"
        />
        <div
          data-tauri-drag-region
          className="font-mono text-[10px] uppercase tracking-[0.16em] text-ink-dim"
        >
          {t(`section.${section}`)}
        </div>
        <div data-tauri-drag-region className="flex-1" />
        {/* Global LLM status pill — visible in every section. The
            user always knows whether the AI is reachable without
            having to drill into Settings or wait for a committee
            to fail. Hidden until the first provider poll resolves. */}
        {activeProviderInfo && (
          <div
            data-tauri-drag-region="false"
            className="flex items-center gap-1.5 border border-border bg-surface/70 px-2 py-1 text-[11px] text-ink-dim"
            title={metaFor(activeProviderInfo.id).name}
          >
            <span
              aria-hidden
              className="grid h-4 w-4 place-content-center rounded bg-slate-400/10 text-[9px] font-semibold text-ink-dim ring-1 ring-border-subtle"
            >
              {metaFor(activeProviderInfo.id).monogram}
            </span>
            <ProviderStatusPill
              status={activeProviderInfo.status}
              size="sm"
            />
          </div>
        )}
        {active ? (
          <div
            data-tauri-drag-region="false"
            className="flex items-center gap-2 border border-border bg-surface/70 px-2.5 py-1 text-[11px] text-ink-dim"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-ok" />
            <span className="truncate max-w-[200px]">{active.name}</span>
            {active.specialty && (
              <>
                <span className="text-ink-faint">·</span>
                <span className="truncate max-w-[160px] text-ink-faint">
                  {active.specialty}
                </span>
              </>
            )}
          </div>
        ) : (
          <span
            data-tauri-drag-region
            className="font-mono text-[10px] uppercase tracking-[0.16em] text-ink-faint"
          >
            {t("app.no_workspace")}
          </span>
        )}
      </header>

      {/* Body row: sidebar + main. */}
      <div className="flex min-h-0 flex-1">
        <Sidebar
          active={section}
          onSelect={setSection}
          workspaceLabel={active ? active.name : null}
          activeProvider={activeProviderInfo}
          providersLoaded={providers !== null}
        />

        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
            {section === "workspaces" && (
              <WorkspacesPage
                activeId={active?.id ?? null}
                onActiveChange={setActive}
              />
            )}
            {section === "knowledge" &&
              (active ? (
                <KnowledgePage workspace={active} />
              ) : (
                <EmptyWorkspaceHint onCreate={() => setSection("workspaces")} />
              ))}
            {section === "cases" &&
              (active ? (
                <CasesPage
                  workspace={active}
                  onGoToSettings={() => setSection("settings")}
                />
              ) : (
                <EmptyWorkspaceHint onCreate={() => setSection("workspaces")} />
              ))}
            {section === "settings" && <SettingsPage />}
          </div>
        </main>
      </div>
    </div>
  );
}

function EmptyWorkspaceHint({ onCreate }: { onCreate: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="mx-auto max-w-md p-12 text-center">
      <div className="mb-2 text-[14px] font-semibold text-ink">
        {t("app.empty_hint_title")}
      </div>
      <p className="mb-4 text-[13px] text-ink-subtle">
        {t("app.empty_hint_body")}
      </p>
      <button
        type="button"
        onClick={onCreate}
        className="rounded-md border border-border bg-surface px-3 py-1.5 text-[13px] text-ink hover:bg-surface-hover"
      >
        {t("app.empty_hint_button")}
      </button>
    </div>
  );
}
