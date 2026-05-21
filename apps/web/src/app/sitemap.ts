import type { MetadataRoute } from "next";
import { routing } from "~/i18n/routing";
import { SITE_URL } from "~/lib/site";

export const dynamic = "force-static";

const PAGES = ["", "disclaimer", "privacy", "terms"] as const;

export default function sitemap(): MetadataRoute.Sitemap {
  const now = new Date();

  return PAGES.flatMap((slug) =>
    routing.locales.map((locale) => {
      const path = slug ? `/${locale}/${slug}/` : `/${locale}/`;
      const url = `${SITE_URL}${path}`;
      return {
        url,
        lastModified: now,
        changeFrequency: "monthly" as const,
        priority: slug === "" ? 1 : 0.5,
        alternates: {
          languages: Object.fromEntries(
            routing.locales.map((l) => {
              const altPath = slug ? `/${l}/${slug}/` : `/${l}/`;
              return [l, `${SITE_URL}${altPath}`];
            }),
          ),
        },
      };
    }),
  );
}
