import { useTranslations } from "next-intl";
import { SectionEyebrow } from "./SectionEyebrow";

export function ProvidersSection() {
  const t = useTranslations("providers");
  const list = t.raw("list") as string[];

  return (
    <section
      id="providers"
      className="bg-paper-subtle border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <SectionEyebrow>Multi-proveedor</SectionEyebrow>
        <h2 className="mt-4 max-w-[18ch] font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
          {t("title")}
        </h2>
        <p className="mt-5 max-w-[58ch] text-[17px] leading-[1.55] text-ink-dim">
          {t("subtitle")}
        </p>

        <ul
          className="mt-14 flex flex-wrap gap-x-7 gap-y-3 font-mono text-[14px] tracking-tight text-ink"
          aria-label={t("title")}
        >
          {list.map((name, i) => (
            <li key={name} className="flex items-center gap-7">
              {i > 0 && (
                <span aria-hidden className="text-ink-faint">
                  ·
                </span>
              )}
              <span>{name}</span>
            </li>
          ))}
        </ul>

        <p className="mt-10 max-w-[58ch] font-mono text-[12.5px] leading-[1.6] text-ink-subtle">
          {t("caption")}
        </p>
      </div>
    </section>
  );
}
