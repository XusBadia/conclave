// Reusable collapsible card section + an inline "what is this?" tooltip.
// The verdict renderer uses these so supporting detail (clinical data, red
// flags, follow-up triggers, applied evidence) starts collapsed and the
// committed recommendation stays the focus of the page.

import { useState, type ReactNode } from "react";
import {
  IconChevronDown,
  IconChevronRight,
  IconInfoCircle,
} from "@tabler/icons-react";

import { CopyButton } from "./banners";

/** Inline info glyph with a native-`title` tooltip — matches the app's
 *  existing title-based tooltip pattern, no extra dependency. */
export function InfoTip({ text }: { text: string }) {
  return (
    <span
      title={text}
      aria-label={text}
      role="img"
      className="inline-flex cursor-help text-ink-faint transition-colors hover:text-ink-subtle"
    >
      <IconInfoCircle size={14} stroke={1.6} aria-hidden />
    </span>
  );
}

const TITLE_CLASS = "text-[11px] uppercase tracking-[0.08em] text-ink-faint";

/** A bordered, collapsible card. The header carries the title, an optional
 *  item count (so a collapsed safety section still signals it has content),
 *  an optional info tooltip and a copy button; the body stays hidden until
 *  the clinician expands it. Collapsed by default. */
export function CollapsibleSection({
  title,
  count,
  helpText,
  copyText,
  defaultOpen = false,
  tone = "default",
  children,
}: {
  title: string;
  count?: number;
  helpText?: string;
  copyText?: string;
  defaultOpen?: boolean;
  /** `warn` tints the count badge so a collapsed-but-non-empty red-flags
   *  section reads as a warning at a glance. */
  tone?: "default" | "warn";
  children: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const Chevron = open ? IconChevronDown : IconChevronRight;
  const countClass =
    tone === "warn" && (count ?? 0) > 0
      ? "border-warn/40 bg-warn/10 text-warn"
      : "border-border-subtle bg-surface text-ink-subtle";
  return (
    <section className="rounded-md border border-border-subtle bg-bg">
      <div className="flex items-center gap-2 px-3 py-2">
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="flex flex-1 items-center gap-2 text-left focus:outline-none focus-visible:ring-conclave"
        >
          <Chevron
            size={14}
            stroke={1.6}
            aria-hidden
            className="shrink-0 text-ink-faint"
          />
          <span className={TITLE_CLASS}>{title}</span>
          {typeof count === "number" && (
            <span
              className={`rounded border px-1.5 py-0.5 text-[10px] font-medium tabular-nums ${countClass}`}
            >
              {count}
            </span>
          )}
        </button>
        {helpText && <InfoTip text={helpText} />}
        {copyText && <CopyButton text={copyText} />}
      </div>
      {open && (
        <div className="border-t border-border-subtle px-3 py-3">{children}</div>
      )}
    </section>
  );
}
