import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  IconBook2,
  IconBriefcase,
  IconLayoutGrid,
  IconSettings,
} from "@tabler/icons-react";

import { cn } from "../lib/cn";
import { Logo } from "./Logo";

export type Section = "workspaces" | "knowledge" | "cases" | "settings";

const ICON_PROPS = { size: 16, stroke: 1.5 } as const;

const items: { id: Section; icon: ReactNode }[] = [
  { id: "workspaces", icon: <IconLayoutGrid {...ICON_PROPS} /> },
  { id: "knowledge", icon: <IconBook2 {...ICON_PROPS} /> },
  { id: "cases", icon: <IconBriefcase {...ICON_PROPS} /> },
  { id: "settings", icon: <IconSettings {...ICON_PROPS} /> },
];

export function Sidebar({
  active,
  onSelect,
  workspaceLabel,
}: {
  active: Section;
  onSelect: (s: Section) => void;
  workspaceLabel: string | null;
}) {
  const { t } = useTranslation();
  return (
    <aside className="flex h-full w-[220px] shrink-0 flex-col border-r border-border bg-bg-subtle">
      <div className="flex items-center gap-2.5 px-4 pb-2 pt-3">
        <Logo size={28} />
        <div className="leading-tight">
          <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-ink">
            {t("app.brand")}
          </div>
          <div className="mt-0.5 text-[10px] uppercase tracking-[0.1em] text-ink-faint">
            {t("sidebar.tagline")}
          </div>
        </div>
      </div>

      {workspaceLabel && (
        <div className="mx-3 mb-2 mt-2 flex items-center gap-2 rounded-md border border-border-subtle bg-surface px-2.5 py-2 text-[12px] text-ink-dim">
          <span className="h-1.5 w-1.5 rounded-full bg-ok" />
          <span className="truncate">{workspaceLabel}</span>
        </div>
      )}

      <nav className="mt-2 flex flex-1 flex-col gap-0.5 px-2">
        {items.map((it) => {
          const selected = active === it.id;
          return (
            <button
              key={it.id}
              type="button"
              onClick={() => onSelect(it.id)}
              className={cn(
                "group flex items-start gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors focus:outline-none focus-visible:ring-conclave",
                selected
                  ? "bg-surface-active text-ink"
                  : "text-ink-dim hover:bg-surface hover:text-ink",
              )}
            >
              <span
                className={cn(
                  "mt-0.5",
                  selected ? "text-ink" : "text-ink-subtle",
                )}
              >
                {it.icon}
              </span>
              <span className="min-w-0">
                <span className="block text-[13px] font-medium">
                  {t(`section.${it.id}`)}
                </span>
                <span
                  className={cn(
                    "mt-0.5 block font-mono text-[10px] uppercase leading-snug tracking-[0.08em]",
                    selected ? "text-ink-subtle" : "text-ink-faint",
                  )}
                >
                  {t(`section_hint.${it.id}`)}
                </span>
              </span>
            </button>
          );
        })}
      </nav>

      <div className="border-t border-border px-4 py-3 text-[11px] leading-snug text-ink-faint">
        {t("sidebar.footer")}
      </div>
    </aside>
  );
}
