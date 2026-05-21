"use client";

import { useLocale, useTranslations } from "next-intl";
import { useTransition } from "react";
import { usePathname, useRouter } from "~/i18n/navigation";
import { routing, type Locale } from "~/i18n/routing";

export function LanguageToggle() {
  const t = useTranslations("ui.language");
  const current = useLocale() as Locale;
  const router = useRouter();
  const pathname = usePathname();
  const [isPending, startTransition] = useTransition();

  return (
    <div
      role="group"
      aria-label={t("toggleLabel")}
      className="inline-flex items-center font-mono text-[12px] uppercase tracking-widest"
    >
      {routing.locales.map((locale, i) => {
        const active = locale === current;
        return (
          <span key={locale} className="contents">
            {i > 0 && (
              <span aria-hidden className="px-1 text-ink-faint">
                /
              </span>
            )}
            <button
              type="button"
              onClick={() => {
                if (active) return;
                startTransition(() => {
                  router.replace(pathname, { locale });
                });
              }}
              aria-current={active ? "true" : undefined}
              disabled={isPending}
              className={[
                "px-0.5 transition-colors duration-200",
                active
                  ? "text-ink"
                  : "text-ink-subtle hover:text-ink focus-visible:text-ink",
              ].join(" ")}
            >
              {t(locale)}
            </button>
          </span>
        );
      })}
    </div>
  );
}
