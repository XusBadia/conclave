import { useEffect, useState } from "react";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Field, Input } from "../components/Field";
import { ipc, type Workspace } from "../lib/ipc";

export function WorkspacesPage({
  activeId,
  onActiveChange,
}: {
  activeId: string | null;
  onActiveChange: (ws: Workspace) => void;
}) {
  const [list, setList] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [name, setName] = useState("");
  const [specialty, setSpecialty] = useState("");
  const [language, setLanguage] = useState("es");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setList(await ipc.listWorkspaces());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const create = async () => {
    if (!name.trim()) return;
    setCreating(true);
    setError(null);
    try {
      const ws = await ipc.createWorkspace(
        name.trim(),
        specialty.trim() || undefined,
        language.trim() || undefined,
      );
      setName("");
      setSpecialty("");
      await refresh();
      await ipc.switchWorkspace(ws.id);
      onActiveChange(ws);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  const switchTo = async (ws: Workspace) => {
    try {
      await ipc.switchWorkspace(ws.id);
      onActiveChange(ws);
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (ws: Workspace) => {
    if (!confirm(`Delete workspace "${ws.name}"? This is irreversible.`))
      return;
    try {
      await ipc.deleteWorkspace(ws.id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-5 p-6 lg:grid-cols-[1fr,380px]">
      <Card>
        <CardHeader
          title="Workspaces"
          subtitle="Per-specialty knowledge bases and rules"
          right={
            <Button size="sm" variant="ghost" onClick={refresh} loading={loading}>
              Refresh
            </Button>
          }
        />
        <CardBody className="p-0">
          {error && (
            <div className="mx-5 mt-4 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
              {error}
            </div>
          )}
          {list.length === 0 && !loading && (
            <div className="px-6 py-10 text-center text-[13px] text-ink-subtle">
              No workspaces yet. Create your first one on the right.
            </div>
          )}
          <ul className="divide-y divide-border-subtle">
            {list.map((ws) => {
              const active = ws.id === activeId;
              return (
                <li
                  key={ws.id}
                  className="flex items-start justify-between gap-4 px-5 py-4"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      {active && (
                        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
                      )}
                      <div className="truncate text-[14px] font-medium text-ink">
                        {ws.name}
                      </div>
                    </div>
                    <div className="mt-0.5 truncate text-[12px] text-ink-faint">
                      <span className="font-mono">{ws.id}</span>
                      {ws.specialty && (
                        <span className="ml-2 rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                          {ws.specialty}
                        </span>
                      )}
                      {ws.language && (
                        <span className="ml-1 rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                          {ws.language}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {!active && (
                      <Button size="sm" onClick={() => switchTo(ws)}>
                        Activate
                      </Button>
                    )}
                    <Button
                      size="sm"
                      variant="danger"
                      onClick={() => remove(ws)}
                    >
                      Delete
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title="Create workspace"
          subtitle="Each workspace gets its own SQLite + vector store"
        />
        <CardBody className="space-y-4">
          <Field label="Name">
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Cardiology — internal protocols"
            />
          </Field>
          <Field label="Specialty (optional)">
            <Input
              value={specialty}
              onChange={(e) => setSpecialty(e.target.value)}
              placeholder="cardiology"
            />
          </Field>
          <Field
            label="Default output language"
            hint="ISO code · used by the verdict prompt"
          >
            <Input
              value={language}
              onChange={(e) => setLanguage(e.target.value)}
              placeholder="es"
            />
          </Field>
          <div className="pt-1">
            <Button
              variant="primary"
              size="md"
              onClick={create}
              loading={creating}
              disabled={!name.trim()}
            >
              Create workspace
            </Button>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}
