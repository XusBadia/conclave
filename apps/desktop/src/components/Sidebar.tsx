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
import { ProviderStatusPill } from "./ProviderStatusPill";
import { metaFor } from "../lib/providers";
import type { ProviderInfo } from "../lib/ipc";

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
  activeProvider,
  providersLoaded,
}: {
  active: Section;
  onSelect: (s: Section) => void;
  workspaceLabel: string | null;
  activeProvider: ProviderInfo | null;
  /** `true` once the first `listProviders()` round-trip has resolved.
   *  Used to decide whether the absence of an active provider means
   *  "still loading" (don't render the empty-state CTA) or "no provider
   *  configured" (render the amber CTA). */
  providersLoaded: boolean;
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

      {/* IA status row — separate from the workspace dot above so the
          existing "workspace active" meaning isn't repurposed silently.
          New surface that mirrors the title bar; if it doesn't fit
          we still have the title bar pill as a fallback. */}
      {activeProvider ? (
        <div className="mx-3 mb-3 flex items-center gap-2 rounded-md border border-border-subtle bg-bg px-2.5 py-1.5 text-[11.5px] text-ink-dim">
          <span
            aria-hidden
            className="grid h-4 w-4 shrink-0 place-content-center rounded bg-slate-400/10 text-[9px] font-semibold text-ink-dim ring-1 ring-border-subtle"
          >
            {metaFor(activeProvider.id).monogram}
          </span>
          <span className="min-w-0 flex-1 truncate">
            {metaFor(activeProvider.id).name}
          </span>
          <ProviderStatusPill status={activeProvider.status} size="sm" />
        </div>
      ) : (
        providersLoaded && (
          // Empty-state CTA. Without this the user can land in
          // Cases/Knowledge with nothing configured and no signal that
          // they need to set up a provider — they'd just see "no
          // results" or generic errors when they try to run anything.
          // Click routes straight to the only place that fixes it.
          <button
            type="button"
            onClick={() => onSelect("settings")}
            className={cn(
              "mx-3 mb-3 flex items-center gap-2 rounded-md border border-warn/40 bg-warn/10 px-2.5 py-1.5 text-left text-[11.5px] text-warn transition no-drag",
              "hover:bg-warn/15 focus:outline-none focus-visible:ring-conclave",
              active === "settings" && "ring-1 ring-warn/60",
            )}
            aria-label={t("settings.sidebar_no_provider_cta")}
            title={t("settings.sidebar_no_provider_hint")}
          >
            <span className="relative grid h-4 w-4 shrink-0 place-content-center">
              <span className="absolute inset-0 m-auto h-1.5 w-1.5 rounded-full bg-warn animate-pulseDot" />
            </span>
            <span className="min-w-0 flex-1 truncate font-medium">
              {t("settings.sidebar_no_provider_cta")}
            </span>
          </button>
        )
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
