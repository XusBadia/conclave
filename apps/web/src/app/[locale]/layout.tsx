import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { hasLocale, NextIntlClientProvider } from "next-intl";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { routing } from "~/i18n/routing";
import { SITE_NAME, SITE_URL, VERSION } from "~/lib/site";
import { Header } from "~/components/Header";
import { Footer } from "~/components/Footer";

export function generateStaticParams() {
  return routing.locales.map((locale) => ({ locale }));
}

export async function generateMetadata(props: {
  params: Promise<{ locale: string }>;
}): Promise<Metadata> {
  const { locale } = await props.params;
  if (!hasLocale(routing.locales, locale)) return {};
  const t = await getTranslations({ locale, namespace: "meta" });

  return {
    title: { default: t("title"), template: `%s — ${SITE_NAME}` },
    description: t("description"),
    alternates: {
      canonical: `${SITE_URL}/${locale}/`,
      languages: {
        es: `${SITE_URL}/es/`,
        en: `${SITE_URL}/en/`,
        "x-default": `${SITE_URL}/es/`,
      },
    },
    openGraph: {
      type: "website",
      url: `${SITE_URL}/${locale}/`,
      siteName: SITE_NAME,
      title: t("title"),
      description: t("description"),
      locale: locale === "es" ? "es_ES" : "en_US",
      alternateLocale: locale === "es" ? ["en_US"] : ["es_ES"],
    },
    twitter: {
      card: "summary_large_image",
      title: t("title"),
      description: t("description"),
    },
    robots: { index: true, follow: true },
  };
}

export default async function LocaleLayout(props: {
  children: React.ReactNode;
  params: Promise<{ locale: string }>;
}) {
  const { locale } = await props.params;
  if (!hasLocale(routing.locales, locale)) notFound();
  setRequestLocale(locale);

  const t = await getTranslations({ locale, namespace: "meta" });

  const jsonLd = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: SITE_NAME,
    description: t("description"),
    applicationCategory: "ProductivityApplication",
    operatingSystem: "macOS, Windows, Linux",
    softwareVersion: VERSION,
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
    author: {
      "@type": "Person",
      name: "Xus Badia",
    },
    inLanguage: ["es", "en"],
    license: "https://github.com/XusBadia/conclave/blob/main/LICENSE-MIT",
    disclaimer:
      locale === "es"
        ? "Conclave no es un dispositivo médico. La autoridad final sobre cualquier decisión clínica siempre corresponde al profesional sanitario."
        : "Conclave is not a medical device. Final authority over any clinical decision always rests with the healthcare professional.",
  };

  return (
    <NextIntlClientProvider>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />
      <Header />
      <main id="main">{props.children}</main>
      <Footer />
    </NextIntlClientProvider>
  );
}
