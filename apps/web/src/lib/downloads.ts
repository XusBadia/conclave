import { REPO_URL, RELEASES_URL, VERSION } from "./site";

export const RELEASES_AVAILABLE = false;

export const downloads = {
  releases: RELEASES_URL,
  source: REPO_URL,
  macos: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_universal.dmg`,
  windows: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_x64-setup.exe`,
  linux: `${REPO_URL}/releases/latest/download/Conclave_${VERSION}_amd64.AppImage`,
} as const;

export type DownloadOS = "macos" | "windows" | "linux";

export function detectOS(userAgent: string): DownloadOS {
  const ua = userAgent.toLowerCase();
  if (ua.includes("mac")) return "macos";
  if (ua.includes("win")) return "windows";
  return "linux";
}
