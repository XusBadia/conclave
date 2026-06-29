import { useTranslations } from "next-intl";
import { Link } from "~/i18n/navigation";
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
