// Confirmation/edit dialogs for the Cases route: date editing sheet plus
// the delete and purge confirmation popovers. Controlled components —
// all state lives in the page.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../components/Button";
import { Field } from "../../components/Field";
import { Popover } from "../../components/Popover";
import { Sheet } from "../../components/Sheet";
import { isoToLocalInput } from "./helpers";

export function EditDateSheet({
  open,
  onOpenChange,
  count,
  initialIso,
  busy,
  error,
  onApply,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  count: number;
  initialIso: string;
  busy: boolean;
  error: string | null;
  onApply: (localValue: string) => void;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState<string>(isoToLocalInput(initialIso));

  // Re-seed the input whenever the sheet (re)opens with a different
  // initial value — without this, opening, closing without saving, and
  // re-opening on a different selection would keep the old value.
  useEffect(() => {
    if (open) setValue(isoToLocalInput(initialIso));
  }, [open, initialIso]);

  const title =
    count > 1
      ? t("cases.edit_date_title_plural", { count })
      : t("cases.edit_date_title");

  return (
    <Sheet open={open} onOpenChange={onOpenChange} title={title}>
      <div className="space-y-4 px-5 py-4">
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger">
            {error}
          </div>
        )}
        <Field label={t("cases.edit_date_field")}>
          <input
            type="datetime-local"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            className="block w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-ink focus:outline-none focus:ring-conclave focus:border-accent"
          />
        </Field>
        <div className="flex justify-end gap-2 pt-2">
          <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            loading={busy}
            disabled={!value}
            onClick={() => onApply(value)}
          >
            {t("cases.edit_date_apply")}
          </Button>
        </div>
      </div>
    </Sheet>
  );
}

export function ConfirmDeletePopover({
  open,
  onOpenChange,
  anchor,
  count,
  busy,
  error,
  onConfirm,
  side = "bottom",
  align = "end",
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  anchor: HTMLElement | null;
  count: number;
  busy: boolean;
  error: string | null;
  onConfirm: () => void;
  side?: "top" | "bottom";
  align?: "start" | "center" | "end";
}) {
  const { t } = useTranslation();
  const title =
    count > 1
      ? t("cases.delete_confirm_title_plural", { count })
      : t("cases.delete_confirm_title");
  const body =
    count > 1
      ? t("cases.delete_confirm_body_plural", { count })
      : t("cases.delete_confirm_body");

  return (
    <Popover
      open={open}
      onOpenChange={onOpenChange}
      anchor={anchor}
      side={side}
      align={align}
      width={320}
      ariaLabel={title}
    >
      <div className="space-y-3 p-4">
        <h3 className="text-[13px] font-semibold text-ink">{title}</h3>
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-2.5 py-1.5 text-[12px] text-danger">
            {t("cases.delete_error", { error })}
          </div>
        )}
        <p className="text-[12.5px] leading-relaxed text-ink-dim">{body}</p>
        <div className="flex justify-end gap-2 pt-1">
          <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" variant="danger" loading={busy} onClick={onConfirm}>
            {t("cases.delete_confirm_apply")}
          </Button>
        </div>
      </div>
    </Popover>
  );
}

export function ConfirmPurgePopover({
  open,
  onOpenChange,
  anchor,
  busy,
  error,
  title,
  lines,
  confirmLabel,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  anchor: HTMLElement | null;
  busy: boolean;
  error: string | null;
  title: string;
  lines: string[];
  confirmLabel: string;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Popover
      open={open}
      onOpenChange={onOpenChange}
      anchor={anchor}
      side="bottom"
      align="end"
      width={360}
      ariaLabel={title}
    >
      <div className="space-y-3 p-4">
        <h3 className="text-[13px] font-semibold text-ink">{title}</h3>
        {error && (
          <div className="rounded-md border border-danger/40 bg-danger/10 px-2.5 py-1.5 text-[12px] text-danger">
            {error}
          </div>
        )}
        <ul className="space-y-1.5 text-[12.5px] leading-relaxed text-ink-dim">
          {lines.map((line, i) => (
            <li key={i} className="flex gap-2">
              <span
                aria-hidden
                className="mt-[7px] h-1 w-1 shrink-0 rounded-full bg-ink-faint"
              />
              <span>{line}</span>
            </li>
          ))}
        </ul>
        <div className="flex justify-end gap-2 pt-1">
          <Button size="sm" variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" variant="danger" loading={busy} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </Popover>
  );
}
