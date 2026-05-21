/**
 * FOUC theme script — runs synchronously in <head> before any paint.
 *
 * Reads localStorage.theme ("light" | "dark" | "auto" or absent) and the
 * media query, then applies `dark` class on <html> before React mounts.
 * Avoids the flash of wrong theme on first load.
 *
 * Returned as a string so layout.tsx can inject it via dangerouslySetInnerHTML.
 */
export const themeScript = `(() => {
  try {
    const saved = localStorage.getItem('theme');
    const mode = saved === 'light' || saved === 'dark' ? saved : null;
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const dark = mode === 'dark' || (mode === null && prefersDark);
    if (dark) document.documentElement.classList.add('dark');
    document.documentElement.dataset.theme = mode ?? 'auto';
  } catch (_) {
    /* localStorage might be unavailable (private mode, embedded WebView, etc.) */
  }
  /* Mark JS as ready so reveal styles can hide content until the
   * IntersectionObserver brings it back. Without this flag, no-JS visitors
   * would see a blank page. */
  document.documentElement.dataset.js = 'ready';
})();`;

export type ThemeMode = "light" | "dark" | "auto";

export function applyTheme(mode: ThemeMode) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (mode === "auto") {
    localStorage.removeItem("theme");
    const prefersDark = window.matchMedia(
      "(prefers-color-scheme: dark)",
    ).matches;
    root.classList.toggle("dark", prefersDark);
  } else {
    localStorage.setItem("theme", mode);
    root.classList.toggle("dark", mode === "dark");
  }
  root.dataset.theme = mode;
}

export function readTheme(): ThemeMode {
  if (typeof document === "undefined") return "auto";
  const stored = localStorage.getItem("theme");
  if (stored === "light" || stored === "dark") return stored;
  return "auto";
}
