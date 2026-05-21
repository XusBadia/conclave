import { useTranslations } from "next-intl";
import { Link } from "~/i18n/navigation";
import { SectionEyebrow } from "./SectionEyebrow";

const CARD_INDICES = [0, 1, 2] as const;

export function PrivacySection() {
  const t = useTranslations("privacy");

  return (
    <section
      id="privacy"
      className="bg-paper-subtle border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <SectionEyebrow>Privacidad por diseño</SectionEyebrow>
        <h2 className="mt-4 max-w-[22ch] font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
          {t("title")}
        </h2>
        <p className="mt-5 max-w-[60ch] text-[17px] leading-[1.55] text-ink-dim">
          {t("subtitle")}
        </p>

        <div className="mt-16 grid grid-cols-1 gap-px bg-hairline border border-hairline md:grid-cols-3">
          {CARD_INDICES.map((i) => (
            <article
              key={i}
              className="bg-paper p-8 flex flex-col gap-3"
            >
              <h3 className="text-[18px] font-medium tracking-tight text-ink">
                {t(`cards.${i}.title`)}
              </h3>
              <p className="text-[14.5px] leading-[1.55] text-ink-dim">
                {t(`cards.${i}.desc`)}
              </p>
            </article>
          ))}
        </div>

        <blockquote className="mt-16 border-l-2 border-ink/30 dark:border-ink/40 pl-6 max-w-[52ch] font-mono text-[14px] leading-[1.65] text-ink-dim">
          {t("disclaimer")}
          <footer className="mt-3">
            <Link
              href="/disclaimer"
              className="text-ink hover:text-accent transition-colors duration-200 underline underline-offset-4 decoration-1 decoration-ink/30"
            >
              {t("disclaimerLink")} ↗
            </Link>
          </footer>
        </blockquote>
      </div>
    </section>
  );
}
