import { useTranslations } from "next-intl";
import { Reveal } from "./Reveal";
import { SectionEyebrow } from "./SectionEyebrow";

type FAQItem = { q: string; a: string };

export function FAQ() {
  const t = useTranslations("faq");
  const items = t.raw("items") as FAQItem[];

  return (
    <section
      id="faq"
      className="bg-paper-subtle border-t border-hairline py-24 sm:py-32 scroll-mt-20"
    >
      <div className="mx-auto max-w-[1200px] px-6 sm:px-8">
        <Reveal>
          <SectionEyebrow>FAQ</SectionEyebrow>
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

        <dl className="mt-14 max-w-[800px] divide-y divide-hairline border-t border-b border-hairline">
          {items.map((item, i) => (
            <Reveal key={i} index={i} stagger={50}>
              <details className="group [&_summary::-webkit-details-marker]:hidden">
                <summary className="cursor-pointer list-none py-6 flex items-start gap-6 transition-colors duration-200 hover:[&_dt]:text-accent">
                  <span className="font-mono text-[11px] leading-[1.8] uppercase tracking-widest text-ink-faint shrink-0 w-8 transition-colors duration-200 group-hover:text-ink-subtle">
                    {String(i + 1).padStart(2, "0")}
                  </span>
                  <dt className="flex-1 text-[17px] font-medium leading-[1.4] tracking-tight text-ink transition-colors duration-200">
                    {item.q}
                  </dt>
                  <span
                    aria-hidden
                    className="shrink-0 mt-1 text-ink-subtle transition-transform duration-300 ease-out group-open:rotate-45 group-hover:text-ink"
                  >
                    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                      <path
                        d="M7 1v12M1 7h12"
                        stroke="currentColor"
                        strokeWidth="1.4"
                        strokeLinecap="round"
                      />
                    </svg>
                  </span>
                </summary>
                <div className="accordion-body">
                  <div className="accordion-inner">
                    <dd className="pl-14 pr-10 pb-7 text-[15px] leading-[1.6] text-ink-dim">
                      {item.a}
                    </dd>
                  </div>
                </div>
              </details>
            </Reveal>
          ))}
        </dl>
      </div>
    </section>
  );
}
