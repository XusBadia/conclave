import { useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { IconLock } from "@tabler/icons-react";

import { Button } from "./Button";
import { ipc } from "../lib/ipc";
import { getLocale } from "../i18n";

export function Onboarding({
  disclaimerEn,
  disclaimerEs,
  onAccepted,
}: {
  disclaimerEn: string;
  disclaimerEs: string;
  onAccepted: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);

  const accept = async () => {
    setBusy(true);
    try {
      await ipc.acceptDisclaimer();
      onAccepted();
    } finally {
      setBusy(false);
    }
  };

  // i18n.language is the source of truth, but we fall back to the helper
  // for hot reloads where the instance hasn't propagated yet.
  const lang = i18n.language || getLocale();
  const disclaimer = lang.startsWith("en") ? disclaimerEn : disclaimerEs;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-bg/85 px-4 pb-4 pt-14 backdrop-blur">
      <div className="animate-in mx-4 max-w-xl border border-border bg-bg-elevated p-7 shadow-soft">
        <div className="mb-5 flex items-center gap-3">
          <div className="grid h-10 w-10 place-content-center border border-ink font-mono text-sm uppercase tracking-[0.1em] text-ink">
            C
          </div>
          <div>
            <div className="font-mono text-[12px] uppercase tracking-[0.14em] text-ink">
              {t("onboarding.welcome")}
            </div>
            <div className="mt-1 text-[12px] text-ink-faint">
              {t("onboarding.subtitle")}
            </div>
          </div>
        </div>

        <h2 className="mb-2 text-[15px] font-semibold text-ink">
          {t("onboarding.header")}
        </h2>
        <p className="mb-3 text-[13px] leading-relaxed text-ink-dim">
          <Trans
            i18nKey="onboarding.body"
            components={[
              <strong key="0" className="text-ink" />,
              <strong key="1" className="text-ink" />,
            ]}
          />
        </p>

        <div className="mb-4 rounded-lg border border-border-subtle bg-bg p-4 text-[12px] leading-relaxed text-ink-subtle">
          {disclaimer}
        </div>

        <div className="mb-5 rounded-lg border border-ok/30 bg-ok/5 p-4 text-[12px] leading-relaxed text-ink-dim">
          <div className="mb-1 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-ok">
            <IconLock aria-hidden="true" size={14} stroke={1.6} />
            {t("onboarding.privacy_title")}
          </div>
          {t("onboarding.privacy_body")}
        </div>

        <p className="mb-5 text-[12px] text-ink-faint">
          {t("onboarding.ack")}
        </p>

        <div className="flex justify-end gap-2">
          <Button variant="primary" size="lg" onClick={accept} loading={busy}>
            {t("onboarding.cta")}
          </Button>
        </div>
      </div>
    </div>
  );
}
