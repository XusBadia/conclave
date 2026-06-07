import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconRestore } from "@tabler/icons-react";

import { Button } from "../components/Button";
import { Field, Input } from "../components/Field";
import { Sheet } from "../components/Sheet";
import { cn } from "../lib/cn";
import {
  DEFAULT_PDF_EXPORT_OPTIONS,
  MAX_HEADER_NOTE,
  loadPdfExportOptions,
  savePdfExportOptions,
  type PdfExportOptions,
} from "./exportOptions";

/** A checkbox row with a bold label and an optional muted hint underneath.
 *  Mirrors the de-identification toggles in Settings for visual consistency. */
function Toggle({
  checked,
  onChange,
  label,
  hint,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  hint?: string;
  disabled?: boolean;
}) {
  return (
    <label
      className={cn(
        "flex cursor-pointer items-start gap-2.5",
        disabled && "cursor-not-allowed opacity-50",
      )}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-4 w-4 shrink-0 accent-accent"
      />
      <span className="min-w-0">
        <span className="block text-[13px] font-medium text-ink-dim">
          {label}
        </span>
        {hint && (
          <span className="mt-0.5 block text-[12px] leading-snug text-ink-faint">
            {hint}
          </span>
        )}
      </span>
    </label>
  );
}

/** Self-contained "set once" options panel for PDF export. Seeds its working
 *  copy from localStorage every time it opens (so a change made elsewhere is
 *  reflected) and persists on every edit. Export actions re-read storage at
 *  click time, so there is no shared state to thread between views. */
export function PdfOptionsSheet({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
}) {
  const { t } = useTranslation();
  const [options, setOptions] = useState<PdfExportOptions>(loadPdfExportOptions);

  // Re-seed from storage on each open so the panel never shows a stale copy.
  useEffect(() => {
    if (open) setOptions(loadPdfExportOptions());
  }, [open]);

  const update = (next: PdfExportOptions) => {
    setOptions(next);
    savePdfExportOptions(next);
  };

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title={t("cases.export_options_title")}
      description={t("cases.export_options_subtitle")}
    >
      <div className="space-y-5 px-5 py-4">
        <div className="space-y-3">
          <Toggle
            checked={options.includeSourceFiles}
            onChange={(v) => update({ ...options, includeSourceFiles: v })}
            label={t("cases.export_options_source_files")}
            hint={t("cases.export_options_source_files_hint")}
          />
          <Toggle
            checked={options.includeAttachmentMeta}
            disabled={!options.includeSourceFiles}
            onChange={(v) => update({ ...options, includeAttachmentMeta: v })}
            label={t("cases.export_options_attachment_meta")}
          />
          <Toggle
            checked={options.includeGenerationMeta}
            onChange={(v) => update({ ...options, includeGenerationMeta: v })}
            label={t("cases.export_options_generation_meta")}
            hint={t("cases.export_options_generation_meta_hint")}
          />
        </div>

        <Field
          label={t("cases.export_options_header_note")}
          hint={t("cases.export_options_header_note_hint")}
        >
          <Input
            value={options.headerNote}
            maxLength={MAX_HEADER_NOTE}
            placeholder={t("cases.export_options_header_note_placeholder")}
            onChange={(e) => update({ ...options, headerNote: e.target.value })}
          />
        </Field>

        <div className="flex items-center justify-between pt-1">
          <button
            type="button"
            onClick={() => update({ ...DEFAULT_PDF_EXPORT_OPTIONS })}
            className="inline-flex items-center gap-1 rounded px-1 text-[12px] text-ink-faint transition hover:text-ink focus:outline-none focus-visible:ring-conclave"
          >
            <IconRestore size={13} stroke={1.6} aria-hidden />
            {t("cases.export_options_restore")}
          </button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => onOpenChange(false)}
          >
            {t("cases.export_options_done")}
          </Button>
        </div>
      </div>
    </Sheet>
  );
}
