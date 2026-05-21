import { useTranslations } from "next-intl";
import { SectionEyebrow } from "./SectionEyebrow";

const STEP_KEYS = ["briefing", "draft", "critique", "verdict"] as const;
type StepKey = (typeof STEP_KEYS)[number];

export function DeliberationDiagram() {
  const t = useTranslations("how");

  return (
    <section
      id="how"
      className="border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <SectionEyebrow>{t("title").split(".")[0]}</SectionEyebrow>
        <h2 className="mt-4 max-w-[24ch] font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
          {t("title")}
        </h2>
        <p className="mt-5 max-w-[58ch] text-[17px] leading-[1.55] text-ink-dim">
          {t("subtitle")}
        </p>

        <ol className="mt-16 grid grid-cols-1 gap-px bg-hairline border border-hairline md:grid-cols-2 lg:grid-cols-4">
          {STEP_KEYS.map((key, i) => (
            <Step key={key} stepKey={key} index={i} />
          ))}
        </ol>
      </div>
    </section>
  );
}

function Step({ stepKey, index }: { stepKey: StepKey; index: number }) {
  const t = useTranslations("how.steps");
  return (
    <li className="relative flex flex-col gap-5 bg-paper p-7 lg:p-8">
      <div className="flex items-baseline justify-between gap-4">
        <span className="font-mono text-[44px] font-light leading-none tracking-tight text-ink">
          {t(`${stepKey}.number`)}
        </span>
        <span className="font-mono text-[10px] uppercase tracking-widest text-ink-faint">
          {t(`${stepKey}.label`)}
        </span>
      </div>
      <div>
        <h3 className="text-[20px] font-medium leading-tight tracking-tight text-ink">
          {t(`${stepKey}.title`)}
        </h3>
        <p className="mt-3 text-[14.5px] leading-[1.55] text-ink-dim">
          {t(`${stepKey}.desc`)}
        </p>
      </div>
      {index < 3 && (
        <span
          aria-hidden
          className="absolute right-[-7px] top-1/2 hidden h-px w-3.5 -translate-y-1/2 bg-hairline-strong lg:block"
        />
      )}
    </li>
  );
}
