import { useEffect, useState } from "react";

import { Onboarding } from "./components/Onboarding";
import { Sidebar, type Section } from "./components/Sidebar";
import { CasesPage } from "./routes/Cases";
import { KnowledgePage } from "./routes/Knowledge";
import { SettingsPage } from "./routes/Settings";
import { WorkspacesPage } from "./routes/Workspaces";
import { ipc, type Workspace } from "./lib/ipc";

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
    <div className="flex h-full w-full">
      {!bootstrap.accepted && (
        <Onboarding
          disclaimer={bootstrap.disclaimer}
          onAccepted={() => setBootstrap({ ...bootstrap, accepted: true })}
        />
      )}

      <Sidebar
        active={section}
        onSelect={setSection}
        workspaceLabel={active ? active.name : null}
      />

      <main className="canvas-grain flex h-full min-w-0 flex-1 flex-col overflow-hidden">
        <div className="titlebar flex shrink-0 items-center px-5">
          <div className="flex-1" />
          {active && (
            <div className="text-[12px] text-ink-faint">
              workspace ·{" "}
              <span className="font-mono text-ink-subtle">{active.id}</span>
            </div>
          )}
        </div>
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
              <EmptyWorkspaceHint
                onCreate={() => setSection("workspaces")}
              />
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
