import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Combobox } from "../components/Combobox";
import { Field, Input } from "../components/Field";
import { Sheet } from "../components/Sheet";
import { specialtyOptions } from "../constants/specialties";
import { ipc, type Workspace } from "../lib/ipc";

export function WorkspacesPage({
  activeId,
  onActiveChange,
}: {
  activeId: string | null;
  onActiveChange: (ws: Workspace) => void;
}) {
  const { t } = useTranslation();
  const [list, setList] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);

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

  const switchTo = async (ws: Workspace) => {
    try {
      await ipc.switchWorkspace(ws.id);
      onActiveChange(ws);
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (ws: Workspace) => {
    if (!confirm(t("workspaces.confirm_delete", { name: ws.name }))) return;
    try {
      await ipc.deleteWorkspace(ws.id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onCreated = async (ws: Workspace) => {
    setCreateOpen(false);
    await refresh();
    try {
      await ipc.switchWorkspace(ws.id);
      onActiveChange(ws);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <>
      <div className="mx-auto w-full max-w-3xl space-y-4 p-6">
        <Card>
          <CardHeader
            title={t("workspaces.page_title")}
            subtitle={t("workspaces.page_subtitle")}
            right={
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={refresh}
                  loading={loading}
                >
                  {t("common.refresh")}
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => setCreateOpen(true)}
                >
                  {t("workspaces.new_button")}
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
            {list.length === 0 && !loading && (
              <div className="px-6 py-12 text-center">
                <p className="text-[13px] text-ink-subtle">
                  {t("workspaces.empty")}
                </p>
                <div className="mt-4">
                  <Button
                    variant="primary"
                    onClick={() => setCreateOpen(true)}
                  >
                    {t("workspaces.new_button")}
                  </Button>
                </div>
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
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        {active && (
                          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
                        )}
                        <div className="truncate text-[14px] font-medium text-ink">
                          {ws.name}
                        </div>
                      </div>
                      <div className="mt-0.5 flex items-center gap-1.5 text-[12px] text-ink-faint">
                        {ws.specialty && (
                          <span className="truncate rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                            {ws.specialty}
                          </span>
                        )}
                        {ws.language && (
                          <span className="rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                            {ws.language}
                          </span>
                        )}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      {!active && (
                        <Button size="sm" onClick={() => switchTo(ws)}>
                          {t("workspaces.activate")}
                        </Button>
                      )}
                      <Button
                        size="sm"
                        variant="danger"
                        onClick={() => remove(ws)}
                      >
                        {t("common.delete")}
                      </Button>
                    </div>
                  </li>
                );
              })}
            </ul>
          </CardBody>
        </Card>
      </div>

      <CreateWorkspaceSheet
        open={createOpen}
        onOpenChange={setCreateOpen}
        onCreated={onCreated}
      />
    </>
  );
}

function CreateWorkspaceSheet({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  onCreated: (ws: Workspace) => void | Promise<void>;
}) {
  const { t, i18n } = useTranslation();
  const [name, setName] = useState("");
  const [specialty, setSpecialty] = useState("");
  const [language, setLanguage] = useState("es");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const specOptions = useMemo(
    () => specialtyOptions(i18n.language),
    [i18n.language],
  );

  // Reset form whenever the sheet opens so the user always starts fresh.
  useEffect(() => {
    if (open) {
      setName("");
      setSpecialty("");
      setLanguage("es");
      setError(null);
      setCreating(false);
    }
  }, [open]);

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
      await onCreated(ws);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("workspaces.create_title")}
      description={t("workspaces.create_subtitle")}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          create();
        }}
        className="space-y-4 p-5"
      >
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
            {error}
          </div>
        )}

        <Field label={t("workspaces.field_name")}>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("workspaces.field_name_placeholder")}
            autoFocus
          />
        </Field>

        <Field
          label={t("workspaces.field_specialty")}
          hint={t("workspaces.field_specialty_hint")}
        >
          <Combobox
            value={specialty}
            onChange={setSpecialty}
            options={specOptions}
            placeholder={t("workspaces.field_specialty_placeholder")}
          />
        </Field>

        <Field
          label={t("workspaces.field_language")}
          hint={t("workspaces.field_language_hint")}
        >
          <Input
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
            placeholder={t("workspaces.field_language_placeholder")}
          />
        </Field>

        <div className="flex items-center justify-end gap-2 pt-2">
          <Button
            type="button"
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={creating}
          >
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            variant="primary"
            loading={creating}
            disabled={!name.trim()}
          >
            {t("workspaces.create_button")}
          </Button>
        </div>
      </form>
    </Sheet>
  );
}
