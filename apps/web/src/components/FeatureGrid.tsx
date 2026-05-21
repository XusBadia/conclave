import { useTranslations } from "next-intl";
import { Reveal } from "./Reveal";
import { SectionEyebrow } from "./SectionEyebrow";

const CARD_INDICES = [0, 1, 2, 3, 4, 5] as const;

export function FeatureGrid() {
  const t = useTranslations("features");

  return (
    <section
      id="features"
      className="border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <Reveal>
          <SectionEyebrow>Capacidades</SectionEyebrow>
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

        <div className="mt-16 grid grid-cols-1 gap-px bg-hairline border border-hairline md:grid-cols-2 lg:grid-cols-3">
          {CARD_INDICES.map((i) => (
            <Reveal key={i} as="article" index={i} stagger={70}>
              <div className="bg-paper p-7 lg:p-8 flex flex-col gap-3 min-h-[180px] h-full transition-colors duration-200 hover:bg-paper-elevated">
                <div className="font-mono text-[10px] uppercase tracking-widest text-ink-faint">
                  / {String(i + 1).padStart(2, "0")}
                </div>
                <h3 className="text-[18px] font-medium tracking-tight text-ink">
                  {t(`cards.${i}.title`)}
                </h3>
                <p className="text-[14.5px] leading-[1.55] text-ink-dim">
                  {t(`cards.${i}.desc`)}
                </p>
              </div>
            </Reveal>
          ))}
        </div>
      </div>
    </section>
  );
}
