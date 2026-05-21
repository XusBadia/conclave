import { setRequestLocale } from "next-intl/server";
import { Hero } from "~/components/Hero";
import { DeliberationDiagram } from "~/components/DeliberationDiagram";
import { PrivacySection } from "~/components/PrivacySection";
import { FeatureGrid } from "~/components/FeatureGrid";
import { ProvidersSection } from "~/components/ProvidersSection";
import { DownloadSection } from "~/components/DownloadSection";
import { FAQ } from "~/components/FAQ";
import type { Locale } from "~/i18n/routing";

export default async function HomePage(props: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await props.params;
  setRequestLocale(locale);

  return (
    <>
      <Hero />
      <DeliberationDiagram />
      <PrivacySection />
      <FeatureGrid />
      <ProvidersSection />
      <DownloadSection />
      <FAQ />
    </>
  );
}
