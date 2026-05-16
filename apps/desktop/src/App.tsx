import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Onboarding } from "./components/Onboarding";
import { Sidebar, type Section } from "./components/Sidebar";
import { CasesPage } from "./routes/Cases";
import { KnowledgePage } from "./routes/Knowledge";
import { SettingsPage } from "./routes/Settings";
import { WorkspacesPage } from "./routes/Workspaces";
import { ipc, type Workspace } from "./lib/ipc";

export function App() {
  const { t } = useTranslation();
  const [section, setSection] = useState<Section>("workspaces");
  const [active, setActive] = useState<Workspace | null>(null);
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
        {active ? (
          <div
            data-tauri-drag-region="false"
            className="flex items-center gap-2 border border-border bg-surface/70 px-2.5 py-1 text-[11px] text-ink-dim"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-ok" />
            <span className="truncate max-w-[200px]">{active.name}</span>
            <span className="text-ink-faint">·</span>
            <span className="font-mono text-ink-faint">{active.id}</span>
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
        />

        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <div className="min-h-0 flex-1 overflow-y-auto">
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
