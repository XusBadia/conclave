import { useTranslations } from "next-intl";
import { Link } from "~/i18n/navigation";
import {
  REPO_URL,
  ISSUES_URL,
  RELEASES_URL,
  LICENSE_URL,
  VERSION,
} from "~/lib/site";
import { Logo, Wordmark } from "./Logo";

export function Footer() {
  const t = useTranslations("footer");
  const year = new Date().getFullYear();

  return (
    <footer className="border-t border-hairline-strong bg-paper">
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8 py-16 sm:py-20">
        <div className="grid grid-cols-2 gap-10 sm:grid-cols-4 lg:gap-16">
          <div className="col-span-2 sm:col-span-1">
            <div className="flex items-center gap-2.5 text-ink">
              <Logo size={22} ariaLabel="Conclave" />
              <Wordmark />
            </div>
            <p className="mt-5 max-w-[28ch] text-[13.5px] leading-[1.55] text-ink-dim">
              {t("tagline")}
            </p>
          </div>

          <FooterColumn heading={t("columns.product")}>
            <FooterLink href="#how">{t("links.howItWorks")}</FooterLink>
            <FooterLink href="#privacy">{t("links.privacy")}</FooterLink>
            <FooterLink href="#features">{t("links.features")}</FooterLink>
            <FooterLink href="#providers">{t("links.providers")}</FooterLink>
            <FooterLink href="#download">{t("links.download")}</FooterLink>
          </FooterColumn>

          <FooterColumn heading={t("columns.legal")}>
            <FooterLink href="/disclaimer" internal>
              {t("links.disclaimer")}
            </FooterLink>
            <FooterLink href="/privacy" internal>
              {t("links.privacyPolicy")}
            </FooterLink>
            <FooterLink href="/terms" internal>
              {t("links.terms")}
            </FooterLink>
          </FooterColumn>

          <FooterColumn heading={t("columns.repo")}>
            <FooterLink href={REPO_URL} external>
              {t("links.github")}
            </FooterLink>
            <FooterLink href={ISSUES_URL} external>
              {t("links.issues")}
            </FooterLink>
            <FooterLink href={RELEASES_URL} external>
              Releases
            </FooterLink>
            <FooterLink href={LICENSE_URL} external>
              {t("links.license")}
            </FooterLink>
          </FooterColumn>
        </div>

        <div className="mt-16 pt-6 border-t border-hairline flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between text-[12.5px] font-mono text-ink-subtle">
          <p>
            {t("year", { year })} · {t("version", { version: VERSION })}
          </p>
          <p className="max-w-[60ch]">{t("copyright")}</p>
        </div>
      </div>
    </footer>
  );
}

function FooterColumn({
  heading,
  children,
}: {
  heading: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <h3 className="font-mono text-[10px] uppercase tracking-widest text-ink-faint">
        {heading}
      </h3>
      <ul className="mt-5 space-y-2.5 text-[13.5px]">{children}</ul>
    </div>
  );
}

function FooterLink({
  href,
  children,
  external,
  internal,
}: {
  href: string;
  children: React.ReactNode;
  external?: boolean;
  internal?: boolean;
}) {
  if (internal) {
    return (
      <li>
        <Link
          href={href as never}
          className="text-ink-dim hover:text-ink transition-colors duration-200"
        >
          {children}
        </Link>
      </li>
    );
  }
  return (
    <li>
      <a
        href={href}
        target={external ? "_blank" : undefined}
        rel={external ? "noopener noreferrer" : undefined}
        className="text-ink-dim hover:text-ink transition-colors duration-200"
      >
        {children}
        {external && <span aria-hidden> ↗</span>}
      </a>
    </li>
  );
}
