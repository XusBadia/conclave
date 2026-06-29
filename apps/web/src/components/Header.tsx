import { useTranslations } from "next-intl";
import { Link } from "~/i18n/navigation";
import { REPO_URL } from "~/lib/site";
import { Logo, Wordmark } from "./Logo";
import { LanguageToggle } from "./LanguageToggle";
import { ThemeToggle } from "./ThemeToggle";

export function Header() {
  const t = useTranslations();
  const nav = [
    { href: "#how", label: t("nav.howItWorks") },
    { href: "#privacy", label: t("nav.privacy") },
    { href: "#features", label: t("nav.features") },
    { href: "#download", label: t("nav.download") },
  ];

  return (
    <header className="sticky top-0 z-40 border-b border-hairline bg-paper/85 backdrop-blur-md backdrop-saturate-150">
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-3 focus:z-50 focus:bg-ink focus:text-paper focus:px-3 focus:py-1 focus:font-mono focus:text-[12px]"
      >
        {t("nav.skipToContent")}
      </a>
      <div className="mx-auto flex h-16 max-w-[1200px] items-center gap-6 px-6 sm:px-8">
        <Link
          href="/"
          className="flex items-center gap-2.5 text-ink hover:opacity-80 transition-opacity"
        >
          <Logo size={22} ariaLabel="Conclave MD" />
          <Wordmark />
        </Link>
        <nav className="hidden md:flex items-center gap-7 text-[13px] text-ink-dim ml-4">
          {nav.map((item) => (
            <a
              key={item.href}
              href={item.href}
              className="hover:text-ink transition-colors duration-200"
            >
              {item.label}
            </a>
          ))}
        </nav>
        <div className="ml-auto flex items-center gap-5">
          <a
            href={REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="GitHub"
            title={t("header.github")}
            className="hidden sm:inline-flex text-ink-dim hover:text-ink transition-colors duration-200"
          >
            <GitHubMark />
          </a>
          <span aria-hidden className="h-3 w-px bg-hairline hidden sm:block" />
          <ThemeToggle />
          <span aria-hidden className="h-3 w-px bg-hairline" />
          <LanguageToggle />
          <span aria-hidden className="h-3 w-px bg-hairline hidden sm:block" />
          <a
            href="#download"
            className="hidden sm:inline-flex h-9 items-center px-4 bg-ink text-paper font-mono text-[12px] uppercase tracking-widest hover:bg-ink-dim transition-colors duration-200"
          >
            {t("header.download")}
          </a>
        </div>
      </div>
    </header>
  );
}

function GitHubMark() {
  return (
    <svg width={18} height={18} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}
