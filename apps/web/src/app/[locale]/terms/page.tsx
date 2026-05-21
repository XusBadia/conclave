import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "~/i18n/navigation";
import type { Locale } from "~/i18n/routing";
import { SITE_URL } from "~/lib/site";

export async function generateMetadata(props: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await props.params;
  const t = await getTranslations({ locale, namespace: "termsPage" });
  return {
    title: t("title"),
    description: t("subtitle"),
    alternates: {
      canonical: `${SITE_URL}/${locale}/terms/`,
      languages: {
        es: `${SITE_URL}/es/terms/`,
        en: `${SITE_URL}/en/terms/`,
      },
    },
  };
}

const SECTION_KEYS = [
  "license",
  "warranty",
  "medical",
  "modifications",
] as const;

export default async function TermsPage(props: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await props.params;
  setRequestLocale(locale);
  const t = await getTranslations({ locale, namespace: "termsPage" });

  return (
    <article className="mx-auto max-w-[760px] px-6 sm:px-8 py-20 sm:py-28">
      <p className="font-mono text-[11px] uppercase tracking-widest text-ink-subtle">
        Legal
      </p>
      <h1 className="mt-4 font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
        {t("title")}
      </h1>
      <p className="mt-5 max-w-[55ch] text-[17px] leading-[1.55] text-ink-dim">
        {t("subtitle")}
      </p>
      <p className="mt-3 font-mono text-[12px] uppercase tracking-widest text-ink-faint">
        {t("lastUpdated")}
      </p>

      <div className="mt-14 space-y-12">
        {SECTION_KEYS.map((key) => (
          <section key={key}>
            <h2 className="text-[20px] font-medium tracking-tight text-ink">
              {t(`sections.${key}.title`)}
            </h2>
            <p className="mt-3 text-[15.5px] leading-[1.7] text-ink-dim">
              {t(`sections.${key}.body`)}
            </p>
          </section>
        ))}
      </div>

      <div className="mt-16 border-t border-hairline pt-8">
        <Link
          href="/"
          className="font-mono text-[13px] text-ink hover:text-accent transition-colors"
        >
          ← {t("backHome")}
        </Link>
      </div>
    </article>
  );
}
