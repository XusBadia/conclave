// i18next bootstrap for the desktop app.
//
// Two locales for now: Spanish (default) and English. The active locale
// lives in `localStorage` under `conclave.locale` so it survives reloads
// without having to roundtrip through the Rust config — the choice is
// pure UI chrome and does not affect verdict generation (each workspace
// owns its own `language` for that).
//
// Detection order:
//   1. `localStorage["conclave.locale"]` if it is `"es"` or `"en"`.
//   2. `navigator.language` if it starts with `"en"` → `"en"`.
//   3. Default → `"es"` (the project's home language).

import i18next from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import es from "./locales/es.json";

export type Locale = "es" | "en";

const STORAGE_KEY = "conclave.locale";

export function detectInitialLocale(): Locale {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === "es" || stored === "en") return stored;
  } catch {
    // localStorage may be unavailable (e.g. private mode); fall through.
  }
  const nav =
    typeof navigator !== "undefined" ? navigator.language ?? "" : "";
  if (nav.toLowerCase().startsWith("en")) return "en";
  return "es";
}

export function setLocale(loc: Locale): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, loc);
  } catch {
    // ignore — best effort.
  }
  void i18next.changeLanguage(loc);
}

export function getLocale(): Locale {
  const lng = i18next.language;
  return lng === "en" ? "en" : "es";
}

void i18next.use(initReactI18next).init({
  resources: {
    es: { translation: es },
    en: { translation: en },
  },
  lng: detectInitialLocale(),
  fallbackLng: "es",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export default i18next;
