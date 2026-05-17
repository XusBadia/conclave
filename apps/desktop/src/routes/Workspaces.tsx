import {
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../components/Button";
import { Card, CardBody, CardHeader } from "../components/Card";
import { Combobox } from "../components/Combobox";
import { Field, Input } from "../components/Field";
import { Sheet } from "../components/Sheet";
import { specialtyOptions } from "../constants/specialties";
import { cn } from "../lib/cn";
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

  const activeWs = list.find((ws) => ws.id === activeId) ?? null;
  const others = list.filter((ws) => ws.id !== activeId);

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

            {list.length === 0 && !loading ? (
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
            ) : (
              <div className="space-y-5 p-5">
                <section>
                  <p className="mb-2 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-faint">
                    {t("workspaces.section_active")}
                  </p>
                  {activeWs ? (
                    <div className="group/hero relative border border-accent/40 bg-accent/[0.05] px-4 py-3.5">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2.5">
                            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
                            <h4 className="truncate text-[15px] font-semibold text-ink">
                              {activeWs.name}
                            </h4>
                            <span className="shrink-0 rounded-pill border border-accent/50 px-2 py-px font-mono text-[9px] uppercase tracking-[0.12em] text-accent">
                              {t("workspaces.active_badge")}
                            </span>
                          </div>
                          {(activeWs.specialty || activeWs.language) && (
                            <div className="mt-2 flex items-center gap-1.5 text-[12px]">
                              {activeWs.specialty && (
                                <span className="truncate rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                                  {activeWs.specialty}
                                </span>
                              )}
                              {activeWs.language && (
                                <span className="rounded bg-surface px-1.5 py-0.5 text-ink-subtle">
                                  {activeWs.language}
                                </span>
                              )}
                            </div>
                          )}
                        </div>
                        <DeleteIconButton
                          ariaLabel={t("workspaces.delete_aria", {
                            name: activeWs.name,
                          })}
                          onClick={() => remove(activeWs)}
                          revealGroup="hero"
                        />
                      </div>
                    </div>
                  ) : (
                    <p className="text-[13px] text-ink-faint">
                      {t("workspaces.no_active")}
                    </p>
                  )}
                </section>

                {others.length > 0 ? (
                  <section>
                    <p className="mb-1 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-faint">
                      {t("workspaces.section_others")}
                      <span className="ml-2 tracking-normal text-ink-faint/70">
                        · {t("workspaces.click_to_switch")}
                      </span>
                    </p>
                    <ul className="divide-y divide-border-subtle border-y border-border-subtle">
                      {others.map((ws) => {
                        const onActivate = () => switchTo(ws);
                        const onKeyDown = (e: KeyboardEvent) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            onActivate();
                          }
                        };
                        return (
                          <li
                            key={ws.id}
                            role="button"
                            tabIndex={0}
                            onClick={onActivate}
                            onKeyDown={onKeyDown}
                            className="group/row flex w-full cursor-pointer items-center justify-between gap-3 px-2 py-3 transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover focus-visible:outline-none"
                          >
                            <div className="min-w-0 flex-1">
                              <div className="truncate text-[13px] font-medium text-ink-dim transition-colors group-hover/row:text-ink">
                                {ws.name}
                              </div>
                              {(ws.specialty || ws.language) && (
                                <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-ink-faint">
                                  {ws.specialty && (
                                    <span className="truncate">
                                      {ws.specialty}
                                    </span>
                                  )}
                                  {ws.specialty && ws.language && (
                                    <span>·</span>
                                  )}
                                  {ws.language && <span>{ws.language}</span>}
                                </div>
                              )}
                            </div>
                            <DeleteIconButton
                              ariaLabel={t("workspaces.delete_aria", {
                                name: ws.name,
                              })}
                              onClick={() => remove(ws)}
                              revealGroup="row"
                            />
                          </li>
                        );
                      })}
                    </ul>
                  </section>
                ) : activeWs ? (
                  <p className="text-[12px] text-ink-faint">
                    {t("workspaces.no_others")}
                  </p>
                ) : null}
              </div>
            )}
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

function DeleteIconButton({
  onClick,
  ariaLabel,
  revealGroup,
}: {
  onClick: () => void;
  ariaLabel: string;
  revealGroup: "hero" | "row";
}) {
  const handleClick = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    onClick();
  };
  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.stopPropagation();
    }
  };
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      onPointerDown={(e) => e.stopPropagation()}
      className={cn(
        "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-pill text-[16px] leading-none text-ink-faint transition-opacity hover:bg-danger/10 hover:text-danger focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-conclave",
        revealGroup === "hero"
          ? "opacity-0 group-hover/hero:opacity-100"
          : "opacity-0 group-hover/row:opacity-100",
      )}
    >
      ×
    </button>
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
