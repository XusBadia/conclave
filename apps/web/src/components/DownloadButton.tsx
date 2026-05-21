"use client";

import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import {
  RELEASES_AVAILABLE,
  detectOS,
  downloads,
  type DownloadOS,
} from "~/lib/downloads";

type DownloadButtonProps = {
  variant?: "primary" | "ghost";
  size?: "sm" | "md" | "lg";
  className?: string;
};

const LABEL_KEY: Record<DownloadOS, "macos" | "windows" | "linux"> = {
  macos: "macos",
  windows: "windows",
  linux: "linux",
};

export function DownloadButton({
  variant = "primary",
  size = "md",
  className,
}: DownloadButtonProps) {
  const t = useTranslations("download");
  const [os, setOs] = useState<DownloadOS>("macos");

  useEffect(() => {
    if (typeof navigator !== "undefined") {
      setOs(detectOS(navigator.userAgent));
    }
  }, []);

  // In "coming soon" mode all buttons route to the GitHub releases page.
  const href = RELEASES_AVAILABLE ? downloads[os] : downloads.releases;
  const label = RELEASES_AVAILABLE
    ? `${t("ctaPrimary").replace("macOS", t(LABEL_KEY[os]))}`
    : t("ctaPrimary");

  const base =
    "inline-flex items-center justify-center gap-2 font-mono text-[13px] tracking-tight transition-colors duration-200 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";
  const sizes: Record<string, string> = {
    sm: "h-9 px-4",
    md: "h-11 px-5",
    lg: "h-12 px-6 text-[14px]",
  };
  const variants: Record<string, string> = {
    primary:
      "bg-ink text-paper hover:bg-ink-dim disabled:opacity-50 disabled:pointer-events-none",
    ghost:
      "border border-ink/15 text-ink hover:border-ink/40 hover:bg-paper-subtle dark:border-ink/20 dark:hover:border-ink/50",
  };

  return (
    <a
      href={href}
      target={RELEASES_AVAILABLE ? undefined : "_blank"}
      rel={RELEASES_AVAILABLE ? undefined : "noopener noreferrer"}
      className={[base, sizes[size], variants[variant], className ?? ""].join(
        " ",
      )}
    >
      <span>{label}</span>
      <ArrowIcon />
    </a>
  );
}

function ArrowIcon() {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M3.5 8h9m0 0L8.5 4m4 4l-4 4"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
