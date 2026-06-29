"use client";

import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { detectOS, downloads, type DownloadOS } from "~/lib/downloads";

type Platform = {
  os: DownloadOS;
  label: string;
  file: string;
};

const PLATFORMS: Platform[] = [
  { os: "macos", label: "macOS", file: ".dmg" },
  { os: "windows", label: "Windows", file: ".msi" },
  { os: "linux", label: "Linux", file: ".AppImage" },
];

// Three explicit per-platform download links. The platform matching the
// visitor's OS is detected client-side and flagged as the recommended one;
// the others stay available so nobody is forced through OS sniffing.
export function PlatformLinks() {
  const t = useTranslations("download");
  const [detected, setDetected] = useState<DownloadOS | null>(null);

  useEffect(() => {
    if (typeof navigator !== "undefined") {
      setDetected(detectOS(navigator.userAgent));
    }
  }, []);

  return (
    <ul className="mt-12 grid gap-3 sm:grid-cols-3">
      {PLATFORMS.map((p) => {
        const isDetected = detected === p.os;
        return (
          <li key={p.os}>
            <a
              href={downloads[p.os]}
              className={[
                "group flex h-full flex-col gap-3 border p-5 transition-colors duration-200",
                isDetected
                  ? "border-ink/40 bg-paper-subtle dark:border-ink/50"
                  : "border-ink/12 hover:border-ink/30 hover:bg-paper-subtle dark:border-ink/15 dark:hover:border-ink/40",
              ].join(" ")}
            >
              <div className="flex items-center justify-between">
                <span className="font-sans text-[17px] font-medium tracking-tight text-ink">
                  {p.label}
                </span>
                {isDetected && (
                  <span className="font-mono text-[10px] uppercase tracking-widest text-accent">
                    {t("detected")}
                  </span>
                )}
              </div>
              <div className="mt-auto flex items-center justify-between font-mono text-[12px] text-ink-subtle">
                <span>{p.file}</span>
                <span className="inline-flex items-center gap-1 text-ink-dim transition-colors group-hover:text-ink">
                  {t("downloadVerb")}
                  <span
                    aria-hidden
                    className="transition-transform duration-200 ease-out group-hover:translate-y-0.5"
                  >
                    ↓
                  </span>
                </span>
              </div>
            </a>
          </li>
        );
      })}
    </ul>
  );
}
