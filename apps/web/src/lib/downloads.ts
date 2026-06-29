import { REPO_URL, RELEASES_URL, VERSION } from "./site";

export const RELEASES_AVAILABLE = true;

// Filenames must match exactly what the Tauri bundler emits (see
// .github/workflows/release.yml). macOS is Apple Silicon only (aarch64),
// Windows ships an .msi, Linux an .AppImage.
export const downloads = {
  releases: RELEASES_URL,
  source: REPO_URL,
  macos: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_aarch64.dmg`,
  windows: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_x64_en-US.msi`,
  linux: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_amd64.AppImage`,
} as const;

export type DownloadOS = "macos" | "windows" | "linux";

export function detectOS(userAgent: string): DownloadOS {
  const ua = userAgent.toLowerCase();
  if (ua.includes("mac")) return "macos";
  if (ua.includes("win")) return "windows";
  return "linux";
}
