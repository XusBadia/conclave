// Provider picker field shared by NewCase and ClassifyDropDialog.
// Moved verbatim from Cases.tsx.

import { useTranslation } from "react-i18next";

import { Button } from "../../components/Button";
import { Field } from "../../components/Field";
import { ProviderStatusPill } from "../../components/ProviderStatusPill";
import { cn } from "../../lib/cn";
import { type ProviderInfo } from "../../lib/ipc";
import { metaFor } from "../../lib/providers";

export function ProviderField({
  providers,
  providerId,
  onChange,
  onGoToSettings,
}: {
  providers: ProviderInfo[];
  providerId: string;
  onChange: (id: string) => void;
  onGoToSettings?: () => void;
}) {
  const { t } = useTranslation();

  if (providers.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border bg-bg-subtle p-4 text-center">
        <div className="text-[13.5px] font-medium text-ink">
          {t("cases.provider_empty_title")}
        </div>
        <p className="mx-auto mt-1 max-w-sm text-[12px] text-ink-subtle">
          {t("cases.provider_empty_body")}
        </p>
        {onGoToSettings && (
          <div className="mt-3">
            <Button size="sm" variant="primary" onClick={onGoToSettings}>
              {t("cases.provider_empty_cta")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (providers.length === 1) {
    const p = providers[0];
    const meta = metaFor(p.id);
    return (
      <Field label={t("cases.field_provider")}>
        <div
          className={cn(
            "flex items-center gap-3 rounded-lg border border-border bg-bg px-3 py-2.5",
          )}
        >
          <span
            aria-hidden
            className="grid h-8 w-8 shrink-0 place-content-center rounded-md bg-slate-400/10 text-[12px] font-semibold text-ink-dim ring-1 ring-border-subtle"
          >
            {meta.monogram}
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <div className="truncate text-[13px] font-medium text-ink">
                {meta.name}
              </div>
              {/* Status pill mirrors what Settings shows — the user
                  sees provider health at the moment of running, not
                  only after a committee fails. */}
              <ProviderStatusPill status={p.status} size="sm" />
            </div>
            <div className="truncate text-[11.5px] text-ink-faint">
              <span className="font-mono">{p.default_model}</span>
              {" · "}
              {meta.authLabel}
            </div>
          </div>
          {onGoToSettings && (
            <button
              type="button"
              onClick={onGoToSettings}
              className="rounded-md px-2 py-1 text-[12px] text-ink-subtle transition hover:bg-surface hover:text-ink focus:outline-none focus-visible:ring-conclave"
            >
              {t("cases.provider_change_link")}
            </button>
          )}
        </div>
      </Field>
    );
  }

  const selected = providers.find((p) => p.id === providerId);
  return (
    <Field
      label={t("cases.field_provider")}
      hint={onGoToSettings ? undefined : t("cases.field_provider_hint")}
    >
      <select
        value={providerId}
        onChange={(e) => onChange(e.target.value)}
        className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink focus:outline-none focus:ring-conclave focus:border-accent"
      >
        {providers.map((p) => {
          const meta = metaFor(p.id);
          return (
            <option key={p.id} value={p.id}>
              {meta.name} · {p.default_model}
            </option>
          );
        })}
      </select>
      {selected && (
        <div className="mt-1.5 flex items-center gap-2 text-[11.5px] text-ink-faint">
          <ProviderStatusPill status={selected.status} size="sm" />
          {onGoToSettings && (
            <button
              type="button"
              onClick={onGoToSettings}
              className="text-[12px] text-ink-faint transition hover:text-ink focus:outline-none focus-visible:underline"
            >
              {t("cases.provider_change_link")}
            </button>
          )}
        </div>
      )}
    </Field>
  );
}
