import { useTranslations } from "next-intl";
import { HeroMockup } from "./HeroMockup";
import { DownloadButton } from "./DownloadButton";
import { downloads } from "~/lib/downloads";

export function Hero() {
  const t = useTranslations("hero");

  return (
    <section className="relative overflow-hidden">
      {/* Desktop mockup — absolute right, bleeds past the viewport edge.
       *  Vertically centered with the text. The mockup wrapper is wider than
       *  its column so the right portion gets clipped by section overflow. */}
      <div
        aria-hidden
        className="pointer-events-none hidden lg:flex absolute inset-y-0 right-0 z-0 w-[52%] items-center pl-24"
      >
        <div className="hero-rise relative w-full" style={{ animationDelay: "180ms" }}>
          <div className="mockup-float lg:w-[108%]">
            <HeroMockup />
          </div>
          {/* Floating mark behind the mockup */}
          <div className="absolute -top-24 -right-24 -z-10 text-ink/5 dark:text-ink/10">
            <svg
              width={360}
              height={360}
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

      {/* Text column — constrained inside max-width container, left half on lg+ */}
      <div className="relative z-10 mx-auto max-w-[1200px] px-6 sm:px-8 py-20 sm:py-28 lg:py-40">
        <div className="lg:max-w-[48%]">
          <p
            className="hero-rise font-mono text-[11px] uppercase tracking-widest text-ink-subtle"
            style={{ animationDelay: "0ms" }}
          >
            {t("eyebrow")}
          </p>
          <h1
            className="hero-rise mt-7 font-sans text-display font-medium leading-[0.95] tracking-tighter text-ink"
            style={{ animationDelay: "100ms" }}
          >
            <span className="block">{t("h1Line1")}</span>
            <span className="block text-ink-subtle">{t("h1Line2")}</span>
          </h1>
          <p
            className="hero-rise mt-7 max-w-[42ch] text-[17px] leading-[1.55] text-ink-dim"
            style={{ animationDelay: "220ms" }}
          >
            {t("dek")}
          </p>
          <div
            className="hero-rise mt-10 flex flex-wrap items-center gap-3"
            style={{ animationDelay: "340ms" }}
          >
            <DownloadButton variant="primary" size="lg" />
            <a
              href={downloads.source}
              target="_blank"
              rel="noopener noreferrer"
              className="group inline-flex h-12 items-center gap-2 px-5 font-mono text-[13px] tracking-tight text-ink hover:text-ink-dim transition-colors duration-200"
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
        </div>
      </div>

      {/* Mobile mockup — stacked below the text */}
      <div className="lg:hidden relative mx-auto max-w-[1200px] px-6 sm:px-8 pb-20 sm:pb-28">
        <div
          className="hero-rise mockup-float"
          style={{ animationDelay: "180ms" }}
        >
          <HeroMockup />
        </div>
      </div>
    </section>
  );
}
