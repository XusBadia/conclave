import type { ReactNode } from "react";

import { cn } from "../lib/cn";

export type Section = "workspaces" | "knowledge" | "cases" | "settings";

const items: { id: Section; label: string; icon: ReactNode; hint: string }[] = [
  {
    id: "workspaces",
    label: "Workspaces",
    hint: "Per-specialty libraries and rules",
    icon: (
      <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
        <path
          d="M3 6.5 6 4l3 2.5M3 6.5v11l3 2.5m-3-13.5h6m-6 11h6m0-13.5 3-2.5 3 2.5m-6 0v11l3 2.5m-3-13.5h6m-6 11h6m0-13.5 3-2.5 3 2.5v11l-3 2.5m0-13.5v11m0 0h-6"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    id: "knowledge",
    label: "Knowledge",
    hint: "Protocols, guidelines, papers",
    icon: (
      <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
        <path
          d="M4 5.5C4 4.67 4.67 4 5.5 4H12v16H5.5A1.5 1.5 0 0 1 4 18.5v-13Zm16 0c0-.83-.67-1.5-1.5-1.5H12v16h6.5a1.5 1.5 0 0 0 1.5-1.5v-13Z"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    id: "cases",
    label: "Cases",
    hint: "Run a virtual committee",
    icon: (
      <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
        <path
          d="M9 4h6l1 3h3a1 1 0 0 1 1 1v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a1 1 0 0 1 1-1h3l1-3Zm3 5.5a3 3 0 0 0-3 3h6a3 3 0 0 0-3-3Zm-3 6h6"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    id: "settings",
    label: "Settings",
    hint: "Providers and routing",
    icon: (
      <svg viewBox="0 0 24 24" fill="none" className="h-4 w-4">
        <path
          d="M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Zm8.4 3.5c0-.4-.04-.79-.1-1.16l2.06-1.61-2-3.46-2.42.97a8 8 0 0 0-2-1.16L15.5 2h-4l-.44 2.58a8 8 0 0 0-2 1.16L6.64 4.77l-2 3.46 2.06 1.6c-.06.38-.1.77-.1 1.17 0 .4.04.79.1 1.16l-2.06 1.61 2 3.46 2.42-.97a8 8 0 0 0 2 1.16L11.5 22h4l.44-2.58a8 8 0 0 0 2-1.16l2.42.97 2-3.46-2.06-1.61c.06-.37.1-.76.1-1.16Z"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
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
  return (
    <aside className="flex h-full w-[220px] shrink-0 flex-col border-r border-border bg-bg-subtle">
      <div className="flex items-center gap-2 px-4 pb-2 pt-3">
        <div className="grid h-7 w-7 place-content-center rounded-md bg-accent text-bg font-semibold">
          C
        </div>
        <div className="leading-tight">
          <div className="text-[13px] font-semibold text-ink">Conclave</div>
          <div className="text-[11px] text-ink-faint">virtual committee</div>
        </div>
      </div>

      {workspaceLabel && (
        <div className="mx-3 mb-2 mt-2 flex items-center gap-2 rounded-md border border-border-subtle bg-surface px-2.5 py-2 text-[12px] text-ink-dim no-drag">
          <span className="h-1.5 w-1.5 rounded-full bg-ok" />
          <span className="truncate">{workspaceLabel}</span>
        </div>
      )}

      <nav className="mt-2 flex flex-1 flex-col gap-0.5 px-2 no-drag">
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
                  selected ? "text-accent" : "text-ink-subtle",
                )}
              >
                {it.icon}
              </span>
              <span className="min-w-0">
                <span className="block text-[13px] font-medium">
                  {it.label}
                </span>
                <span
                  className={cn(
                    "block text-[11px] leading-snug",
                    selected ? "text-ink-subtle" : "text-ink-faint",
                  )}
                >
                  {it.hint}
                </span>
              </span>
            </button>
          );
        })}
      </nav>

      <div className="border-t border-border-subtle px-4 py-3 text-[11px] leading-snug text-ink-faint">
        Not a medical device. Decisions remain with the clinician.
      </div>
    </aside>
  );
}
