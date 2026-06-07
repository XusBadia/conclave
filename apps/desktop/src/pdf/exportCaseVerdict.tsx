import type { TFunction } from "i18next";
import type { CaseDetail } from "../lib/ipc";
import { buildPdfFilename } from "./filename";
import { DEFAULT_PDF_EXPORT_OPTIONS, type PdfExportOptions } from "./exportOptions";

export interface ExportCaseVerdictResult {
  /** `true` when the PDF was written to disk, `false` when the user cancelled
   *  the save dialog. Callers use this to decide whether to show a success
   *  indicator. */
  saved: boolean;
  /** Absolute path the user picked. `undefined` on cancellation. */
  path?: string;
}

/** Generates a PDF of the case verdict and prompts the user for a save
 *  location via the OS-native dialog. Returns silently on cancellation.
 *
 *  Heavy modules (`@react-pdf/renderer`, the PDF component) are loaded lazily
 *  to keep them out of the main bundle — they only ship to users who actually
 *  export. */
export async function exportCaseVerdictToPDF(
  detail: CaseDetail,
  t: TFunction,
  locale: string,
  options: PdfExportOptions = DEFAULT_PDF_EXPORT_OPTIONS,
): Promise<ExportCaseVerdictResult> {
  if (!detail.verdict) {
    throw new Error(t("cases.no_verdict") as string);
  }

  const [{ pdf }, { default: CaseVerdictPDF }, { save }, { writeFile }] =
    await Promise.all([
      import("@react-pdf/renderer"),
      import("./CaseVerdictPDF"),
      import("@tauri-apps/plugin-dialog"),
      import("@tauri-apps/plugin-fs"),
    ]);

  const blob = await pdf(
    <CaseVerdictPDF detail={detail} t={t} locale={locale} options={options} />,
  ).toBlob();

  const defaultPath = buildPdfFilename(detail, t("cases.pdf.filename_prefix") as string);
  const path = await save({
    defaultPath,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });

  if (path === null || path === undefined) {
    return { saved: false };
  }

  const bytes = new Uint8Array(await blob.arrayBuffer());
  await writeFile(path, bytes);
  return { saved: true, path };
}
