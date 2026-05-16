import { useEffect, useState } from "react";

// Theme controller — mirrors the shape of i18n.ts. The user choice is one
// of three modes; "system" defers to the OS via prefers-color-scheme.
// The resolved value ("light" | "dark") drives the `dark` class on
// <html>, which Tailwind reads via darkMode: "class".

export type Theme = "system" | "light" | "dark";

const STORAGE_KEY = "conclave.theme";

function isTheme(v: string | null): v is Theme {
  return v === "system" || v === "light" || v === "dark";
}

function mql(): MediaQueryList {
  return window.matchMedia("(prefers-color-scheme: dark)");
}

export function getStoredTheme(): Theme {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (isTheme(v)) return v;
  } catch {
    /* localStorage unavailable */
  }
  return "system";
}

export function resolveTheme(t: Theme): "light" | "dark" {
  if (t === "system") return mql().matches ? "dark" : "light";
  return t;
}

export function applyTheme(t: Theme): void {
  document.documentElement.classList.toggle("dark", resolveTheme(t) === "dark");
}

export function setTheme(t: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, t);
  } catch {
    /* localStorage unavailable */
  }
  applyTheme(t);
}

export function subscribeSystemTheme(onChange: () => void): () => void {
  const m = mql();
  const handler = () => onChange();
  m.addEventListener("change", handler);
  return () => m.removeEventListener("change", handler);
}

export function useTheme(): readonly [Theme, (t: Theme) => void] {
  const [theme, setLocal] = useState<Theme>(getStoredTheme);

  useEffect(() => {
    if (theme !== "system") return;
    return subscribeSystemTheme(() => applyTheme("system"));
  }, [theme]);

  const set = (t: Theme) => {
    setTheme(t);
    setLocal(t);
  };

  return [theme, set] as const;
}
