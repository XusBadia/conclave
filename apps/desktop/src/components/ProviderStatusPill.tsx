// Single source of truth for the visual representation of a
// `ProviderStatus`. Rendered in every UI surface where the user can
// see *which* LLM is active and *how healthy* it is right now:
// Settings (active card), Cases (form strip + deliberation overlay
// header), the global title bar, and the sidebar workspace card.
//
// Centralising the status → (color, label) mapping here is what
// guarantees that "Sesión caducada" doesn't appear in Settings while
// the title bar still shows "Listo".

import { useTranslation } from "react-i18next";

import { cn } from "../lib/cn";
import { statusLabelKey, statusTone } from "../lib/providerStatus";
import type { ProviderStatus } from "../lib/ipc";

type Size = "sm" | "md";

const TONE_CLASSES: Record<"ok" | "warn" | "neutral", string> = {
  ok: "bg-ok/15 text-ok",
  warn: "bg-warn/15 text-warn",
  neutral: "bg-slate-400/15 text-ink-faint",
};

const SIZE_CLASSES: Record<Size, string> = {
  sm: "px-1.5 py-0.5 text-[9.5px]",
  md: "px-1.5 py-0.5 text-[10px]",
};

export function ProviderStatusPill({
  status,
  size = "md",
  title,
  className,
}: {
  status: ProviderStatus;
  size?: Size;
  title?: string;
  className?: string;
}) {
  const { t } = useTranslation();
  const tone = statusTone(status);
  const labelKey = statusLabelKey(status);
  return (
    <span
      className={cn(
        "rounded font-medium uppercase tracking-wide whitespace-nowrap",
        TONE_CLASSES[tone],
        SIZE_CLASSES[size],
        className,
      )}
      title={title}
    >
      {t(labelKey)}
    </span>
  );
}
