import { useTranslations } from "next-intl";
import { HeroMockup } from "./HeroMockup";
import { DownloadButton } from "./DownloadButton";
import { downloads } from "~/lib/downloads";

export function Hero() {
  const t = useTranslations("hero");

  return (
    <section className="relative overflow-hidden">
      <div className="mx-auto grid max-w-[1200px] grid-cols-1 items-center gap-12 px-6 py-20 sm:px-8 sm:py-28 lg:grid-cols-[1.05fr_1fr] lg:gap-16 lg:py-36">
        <div className="relative z-10">
          <p className="font-mono text-[11px] uppercase tracking-widest text-ink-subtle">
            {t("eyebrow")}
          </p>
          <h1 className="mt-7 font-sans text-display font-medium leading-[0.95] tracking-tighter text-ink">
            <span className="block">{t("h1Line1")}</span>
            <span className="block text-ink-subtle">{t("h1Line2")}</span>
          </h1>
          <p className="mt-7 max-w-[42ch] text-[17px] leading-[1.55] text-ink-dim">
            {t("dek")}
          </p>
          <div className="mt-10 flex flex-wrap items-center gap-3">
            <DownloadButton variant="primary" size="lg" />
            <a
              href={downloads.source}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex h-12 items-center px-5 font-mono text-[13px] tracking-tight text-ink hover:text-ink-dim transition-colors duration-200"
            >
              {t("ctaSecondary")} ↗
            </a>
          </div>
        </div>
        <div className="relative">
          <HeroMockup />
          {/* Floating mark behind mockup */}
          <div
            aria-hidden
            className="pointer-events-none absolute -right-16 -top-16 -z-10 text-ink/5 dark:text-ink/10"
          >
            <svg
              width={320}
              height={320}
              viewBox="0 0 64 64"
              fill="currentColor"
            >
              <circle cx="32" cy="32" r="8" />
              <circle cx="32" cy="8" r="5" />
              <circle cx="9.17" cy="24.58" r="5" />
              <circle cx="17.89" cy="51.42" r="5" />
              <circle cx="46.11" cy="51.42" r="5" />
              <circle cx="54.83" cy="24.58" r="5" />
            </svg>
          </div>
        </div>
      </div>
    </section>
  );
}
