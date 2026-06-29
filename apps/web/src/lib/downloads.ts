import { REPO_URL, RELEASES_URL, VERSION } from "./site";

export const RELEASES_AVAILABLE = true;

// Filenames must match exactly what the Tauri bundler emits (see
// .github/workflows/release.yml). macOS is Apple Silicon only (aarch64),
// Windows ships an .msi, Linux an .AppImage.
//
// productName is "Conclave MD" (with a space), so the bundler emits filenames
// like `Conclave MD_0.1.0_aarch64.dmg`. GitHub replaces spaces in release
// asset names with dots on upload, so the public download URL is
// `Conclave.MD_…` — NOT a space and NOT %20. Verify against a real draft
// release before publishing; a mismatch 404s silently.
export const downloads = {
  releases: RELEASES_URL,
  source: REPO_URL,
  macos: `${REPO_URL}/releases/latest/download/Conclave.MD_${VERSION}_aarch64.dmg`,
  windows: `${REPO_URL}/releases/latest/download/Conclave.MD_${VERSION}_x64_en-US.msi`,
  linux: `${REPO_URL}/releases/latest/download/Conclave.MD_${VERSION}_amd64.AppImage`,
} as const;

export type DownloadOS = "macos" | "windows" | "linux";

export function detectOS(userAgent: string): DownloadOS {
  const ua = userAgent.toLowerCase();
  if (ua.includes("mac")) return "macos";
  if (ua.includes("win")) return "windows";
  return "linux";
}
