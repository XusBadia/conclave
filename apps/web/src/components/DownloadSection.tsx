import { useTranslations } from "next-intl";
import { DownloadButton } from "./DownloadButton";
import { Reveal } from "./Reveal";
import { SectionEyebrow } from "./SectionEyebrow";
import { downloads, RELEASES_AVAILABLE } from "~/lib/downloads";

export function DownloadSection() {
  const t = useTranslations("download");

  return (
    <section
      id="download"
      className="relative overflow-hidden border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <Reveal>
          <SectionEyebrow>{t("eyebrow")}</SectionEyebrow>
        </Reveal>
        <Reveal delay={80}>
          <h2 className="mt-4 max-w-[22ch] font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
            {t("title")}
          </h2>
        </Reveal>
        <Reveal delay={160}>
          <p className="mt-5 max-w-[58ch] text-[17px] leading-[1.55] text-ink-dim">
            {t("subtitle")}
          </p>
        </Reveal>

        <Reveal delay={240}>
          <div className="mt-12 flex flex-wrap items-center gap-3">
            <DownloadButton variant="primary" size="lg" />
            <a
              href={downloads.source}
              target="_blank"
              rel="noopener noreferrer"
              className="group inline-flex h-12 items-center gap-2 px-5 border border-ink/15 hover:border-ink/40 hover:bg-paper-subtle dark:border-ink/20 dark:hover:border-ink/50 font-mono text-[13px] tracking-tight text-ink transition-colors duration-200"
            >
              {t("ctaSecondary")}
              <span
                aria-hidden
                className="inline-block transition-transform duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
              >
                ↗
              </span>
            </a>
          </div>
        </Reveal>

        <Reveal delay={320}>
          <div className="mt-10 max-w-[58ch]">
            <p className="font-mono text-[11px] uppercase tracking-widest text-ink-subtle">
              {t("requirements.title")}
            </p>
            <ul className="mt-3 space-y-1.5 font-mono text-[12.5px] leading-[1.6] text-ink-dim">
              <li>{t("requirements.macos")}</li>
              <li>{t("requirements.ai")}</li>
              <li>{t("requirements.other")}</li>
            </ul>
          </div>
        </Reveal>

        {RELEASES_AVAILABLE && (
          <div className="mt-10 inline-flex flex-wrap gap-2 font-mono text-[12px] uppercase tracking-widest text-ink-subtle">
            <span>{t("macos")}</span>
            <span aria-hidden>·</span>
            <span>{t("windows")}</span>
            <span aria-hidden>·</span>
            <span>{t("linux")}</span>
          </div>
        )}

        <Reveal delay={400}>
          <p className="mt-10 max-w-[52ch] font-mono text-[12.5px] leading-[1.6] text-ink-subtle">
            {t("footnote")}
          </p>
        </Reveal>
      </div>

      {/* Background mark */}
      <div
        aria-hidden
        className="pointer-events-none absolute -bottom-32 -right-32 -z-0 text-ink/[0.035] dark:text-ink/[0.06]"
      >
        <svg
          width={480}
          height={480}
          viewBox="0 0 64 64"
          fill="none"
        >
          <path
            d="M43 20.5C39.9 17.4 35.8 15.5 31.4 15.5C22.5 15.5 15.5 22.8 15.5 32C15.5 41.2 22.5 48.5 31.4 48.5C35.8 48.5 39.9 46.6 43 43.5"
            stroke="currentColor"
            strokeWidth="7.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <circle cx="43.5" cy="20.5" r="4.75" fill="currentColor" />
          <circle cx="43.5" cy="43.5" r="4.75" fill="currentColor" />
          <circle cx="32" cy="32" r="4.25" fill="currentColor" />
        </svg>
      </div>
    </section>
  );
}
