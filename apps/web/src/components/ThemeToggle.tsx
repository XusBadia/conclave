"use client";

import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { applyTheme, readTheme, type ThemeMode } from "~/lib/theme";

const ORDER: ThemeMode[] = ["light", "dark", "auto"];

export function ThemeToggle() {
  const t = useTranslations("ui.theme");
  const [mode, setMode] = useState<ThemeMode>("auto");
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMode(readTheme());
    setMounted(true);
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      if (readTheme() === "auto") applyTheme("auto");
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  function cycle() {
    const next = ORDER[(ORDER.indexOf(mode) + 1) % ORDER.length];
    applyTheme(next);
    setMode(next);
  }

  // Render a stable placeholder during SSR / before hydration to avoid mismatch.
  const label = mounted ? t(mode) : t("auto");

  return (
    <button
      type="button"
      onClick={cycle}
      aria-label={t("toggleLabel")}
      title={t("toggleLabel")}
      className="font-mono text-[12px] uppercase tracking-widest text-ink-subtle hover:text-ink focus-visible:text-ink transition-colors duration-200"
    >
      {label}
    </button>
  );
}
