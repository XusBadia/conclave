import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "~/i18n/navigation";
import type { Locale } from "~/i18n/routing";
import { SITE_URL } from "~/lib/site";

export async function generateMetadata(props: {
  params: Promise<{ locale: Locale }>;
}): Promise<Metadata> {
  const { locale } = await props.params;
  const t = await getTranslations({ locale, namespace: "disclaimerPage" });
  return {
    title: t("title"),
    description: t("subtitle"),
    alternates: {
      canonical: `${SITE_URL}/${locale}/disclaimer/`,
      languages: {
        es: `${SITE_URL}/es/disclaimer/`,
        en: `${SITE_URL}/en/disclaimer/`,
      },
    },
  };
}

const TEXT = {
  es: [
    "Conclave es una herramienta de apoyo a la decisión clínica diseñada para uso exclusivo de profesionales sanitarios cualificados. No constituye un producto sanitario en el sentido del Reglamento (UE) 2017/745, no posee marcado CE y no ha sido evaluada ni aprobada por ninguna autoridad reguladora para fines diagnósticos o terapéuticos.",
    "Conclave no sustituye el juicio clínico del facultativo, no establece diagnósticos, y no debe utilizarse como única base para ninguna decisión clínica. Las recomendaciones que genera son orientativas, son producidas por modelos de lenguaje que pueden cometer errores u omisiones, y dependen de la calidad, exactitud y completitud de la información aportada por el usuario.",
    "La responsabilidad final de cualquier decisión clínica recae exclusivamente en el profesional sanitario tratante. El usuario debe contrastar todas las recomendaciones con la evidencia vigente, los protocolos locales y las circunstancias clínicas concretas de cada paciente.",
    "El usuario es responsable de cumplir toda la normativa aplicable de protección de datos y reglas éticas, incluido el Reglamento General de Protección de Datos (RGPD) de la UE y, cuando proceda, la LOPDGDD. Conclave proporciona herramientas de de-identificación, pero la correcta configuración y uso de dichas herramientas es responsabilidad del usuario. El usuario no debe introducir datos identificables de pacientes en Conclave sin la base legal adecuada.",
    "Conclave se proporciona «tal cual», sin garantía de ningún tipo, expresa o implícita. Los desarrolladores no aceptan responsabilidad alguna por daños derivados del uso o uso indebido del software.",
    "Al utilizar Conclave, el usuario acepta estos términos.",
  ],
  en: [
    "Conclave is a clinical decision support tool intended for use by qualified healthcare professionals. It is not a medical device within the meaning of Regulation (EU) 2017/745 (MDR), is not CE-marked, and has not been evaluated or approved by any regulatory authority for diagnostic or therapeutic use.",
    "Conclave does not replace clinical judgement, does not establish diagnoses, and must not be used as the sole basis for any clinical decision. The recommendations it generates are advisory, are produced by large language models that can make errors or omissions, and depend on the quality, accuracy and completeness of the information provided by the user.",
    "Final responsibility for any clinical decision rests exclusively with the treating healthcare professional. The user must verify all recommendations against current evidence, local protocols, and the specific clinical circumstances of each patient.",
    "Users are responsible for complying with all applicable data protection laws and ethical rules, including the EU General Data Protection Regulation (GDPR) and, where applicable, Spain's LOPDGDD. Conclave provides de-identification tools, but the correct configuration and use of those tools is the user's responsibility. Users must not introduce identifiable patient data into Conclave without the appropriate legal basis.",
    "Conclave is provided \"as is\", without warranty of any kind, express or implied. The developers accept no liability for any damages arising from the use or misuse of the software.",
    "By using Conclave, the user accepts these terms.",
  ],
};

export default async function DisclaimerPage(props: {
  params: Promise<{ locale: Locale }>;
}) {
  const { locale } = await props.params;
  setRequestLocale(locale);
  const t = await getTranslations({ locale, namespace: "disclaimerPage" });
  const paragraphs = TEXT[locale];

  return (
    <article className="mx-auto max-w-[760px] px-6 sm:px-8 py-20 sm:py-28">
      <p className="font-mono text-[11px] uppercase tracking-widest text-ink-subtle">
        {locale === "es" ? "Legal" : "Legal"}
      </p>
      <h1 className="mt-4 font-sans text-display-2 font-medium leading-[1] tracking-tighter text-ink">
        {t("title")}
      </h1>
      <p className="mt-5 max-w-[55ch] text-[17px] leading-[1.55] text-ink-dim">
        {t("subtitle")}
      </p>
      <div className="mt-12 space-y-6 text-[15.5px] leading-[1.7] text-ink-dim">
        {paragraphs.map((p, i) => (
          <p key={i}>{p}</p>
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
