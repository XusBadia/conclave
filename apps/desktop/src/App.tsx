import { useEffect, useState } from "react";

import { Onboarding } from "./components/Onboarding";
import { Sidebar, type Section } from "./components/Sidebar";
import { CasesPage } from "./routes/Cases";
import { KnowledgePage } from "./routes/Knowledge";
import { SettingsPage } from "./routes/Settings";
import { WorkspacesPage } from "./routes/Workspaces";
import { ipc, type Workspace } from "./lib/ipc";

const SECTION_LABEL: Record<Section, string> = {
  workspaces: "Workspaces",
  knowledge: "Knowledge",
  cases: "Cases",
  settings: "Settings",
};

export function App() {
  const [section, setSection] = useState<Section>("workspaces");
  const [active, setActive] = useState<Workspace | null>(null);
  const [bootstrap, setBootstrap] = useState<{
    accepted: boolean;
    disclaimer: string;
  } | null>(null);

  useEffect(() => {
    (async () => {
      const status = await ipc.onboardingStatus();
      setBootstrap({ accepted: status.accepted, disclaimer: status.disclaimer });
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
        loading…
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
      {!bootstrap.accepted && (
        <Onboarding
          disclaimer={bootstrap.disclaimer}
          onAccepted={() => setBootstrap({ ...bootstrap, accepted: true })}
        />
      )}

      {/* macOS overlay title bar — spans the full window so the user can
          drag from anywhere and traffic lights stay out of content. */}
      <header className="titlebar titlebar-pad-mac flex items-center gap-3 pr-5 text-[12px] text-ink-faint">
        <div className="text-[13px] font-semibold tracking-tight text-ink">
          Conclave
        </div>
        <span className="text-ink-faint/60">·</span>
        <div className="text-[12px] uppercase tracking-[0.08em] text-ink-subtle">
          {SECTION_LABEL[section]}
        </div>
        <div className="flex-1" />
        {active ? (
          <div className="flex items-center gap-2 rounded-md border border-border-subtle bg-surface/70 px-2.5 py-1 text-[11px] text-ink-dim">
            <span className="h-1.5 w-1.5 rounded-full bg-ok" />
            <span className="truncate max-w-[200px]">{active.name}</span>
            <span className="text-ink-faint">·</span>
            <span className="font-mono text-ink-faint">{active.id}</span>
          </div>
        ) : (
          <span className="text-ink-faint">no workspace</span>
        )}
      </header>

      {/* Body row: sidebar + main. */}
      <div className="flex min-h-0 flex-1">
        <Sidebar
          active={section}
          onSelect={setSection}
          workspaceLabel={active ? active.name : null}
        />

        <main className="canvas-grain flex min-w-0 flex-1 flex-col overflow-hidden">
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
                <CasesPage workspace={active} />
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
  return (
    <div className="mx-auto max-w-md p-12 text-center">
      <div className="mb-2 text-[14px] font-semibold text-ink">
        No active workspace
      </div>
      <p className="mb-4 text-[13px] text-ink-subtle">
        Pick or create a workspace first — every document and case lives inside
        one.
      </p>
      <button
        type="button"
        onClick={onCreate}
        className="rounded-md border border-border bg-surface px-3 py-1.5 text-[13px] text-ink hover:bg-surface-hover"
      >
        Open Workspaces
      </button>
    </div>
  );
}
