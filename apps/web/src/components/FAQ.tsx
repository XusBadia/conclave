import { useTranslations } from "next-intl";
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
        <SectionEyebrow>FAQ</SectionEyebrow>
        <h2 className="mt-4 max-w-[22ch] font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
          {t("title")}
        </h2>
        <p className="mt-5 max-w-[58ch] text-[17px] leading-[1.55] text-ink-dim">
          {t("subtitle")}
        </p>

        <dl className="mt-14 max-w-[800px] divide-y divide-hairline border-t border-b border-hairline">
          {items.map((item, i) => (
            <details
              key={i}
              className="group [&_summary::-webkit-details-marker]:hidden"
            >
              <summary className="cursor-pointer list-none py-6 flex items-start gap-6">
                <span className="font-mono text-[11px] leading-[1.8] uppercase tracking-widest text-ink-faint shrink-0 w-8">
                  {String(i + 1).padStart(2, "0")}
                </span>
                <dt className="flex-1 text-[17px] font-medium leading-[1.4] tracking-tight text-ink">
                  {item.q}
                </dt>
                <span
                  aria-hidden
                  className="shrink-0 mt-1 text-ink-subtle transition-transform duration-200 group-open:rotate-45"
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
              <dd className="pl-14 pr-10 pb-7 text-[15px] leading-[1.6] text-ink-dim">
                {item.a}
              </dd>
            </details>
          ))}
        </dl>
      </div>
    </section>
  );
}
