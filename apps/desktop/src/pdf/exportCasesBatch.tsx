import type { TFunction } from "i18next";

import { ipc } from "../lib/ipc";
import { buildPdfFilename } from "./filename";
import { DEFAULT_PDF_EXPORT_OPTIONS, type PdfExportOptions } from "./exportOptions";

export interface BatchExportProgress {
  /** Cases processed so far (written + skipped). */
  done: number;
  /** Total cases in the batch. */
  total: number;
}

export interface BatchExportResult {
  /** Number of PDFs actually written to disk. */
  saved: number;
  /** Case ids that produced no PDF — no verdict on file, or a render/IO
   *  error. The caller reports the count so a silent gap can't look like a
   *  complete export. */
  skipped: string[];
  /** `true` when the user dismissed the folder picker — nothing was written
   *  and there is no summary to show. */
  cancelled: boolean;
  /** `true` when the caller aborted mid-run via the `AbortSignal`. Any PDFs
   *  written before the abort are still on disk (`saved` counts them). */
  aborted: boolean;
  /** Absolute directory the PDFs were written to. Undefined when cancelled. */
  dir?: string;
}

/** Makes `name` unique within `used` by inserting ` (2)`, ` (3)`… before the
 *  extension, then records the result in `used`. Two cases that share a
 *  patient label and date would otherwise collide and overwrite each other. */
function uniqueName(name: string, used: Set<string>): string {
  if (!used.has(name)) {
    used.add(name);
    return name;
  }
  const dot = name.lastIndexOf(".");
  const stem = dot === -1 ? name : name.slice(0, dot);
  const ext = dot === -1 ? "" : name.slice(dot);
  let n = 2;
  let candidate = `${stem} (${n})${ext}`;
  while (used.has(candidate)) {
    n += 1;
    candidate = `${stem} (${n})${ext}`;
  }
  used.add(candidate);
  return candidate;
}

/** Exports one PDF per case into a user-picked folder.
 *
 *  The case list only holds `CaseRecord`s, so each case is re-fetched with
 *  `showCase` to get the verdict + attachments the PDF needs. Cases without a
 *  verdict are skipped (not every case has been run). Heavy modules
 *  (`@react-pdf/renderer`, the PDF component, Tauri FS/dialog) are loaded
 *  lazily — they only ship to users who actually export.
 *
 *  Returns `{ cancelled: true }` immediately if the user dismisses the folder
 *  picker. Runs sequentially so progress is monotonic and a large selection
 *  doesn't fan out dozens of concurrent renders. */
export async function exportCasesToFolder(
  workspaceId: string,
  caseIds: string[],
  t: TFunction,
  locale: string,
  options: PdfExportOptions = DEFAULT_PDF_EXPORT_OPTIONS,
  onProgress?: (p: BatchExportProgress) => void,
  signal?: AbortSignal,
): Promise<BatchExportResult> {
  const [{ pdf }, { default: CaseVerdictPDF }, { open }, { writeFile }, { join }] =
    await Promise.all([
      import("@react-pdf/renderer"),
      import("./CaseVerdictPDF"),
      import("@tauri-apps/plugin-dialog"),
      import("@tauri-apps/plugin-fs"),
      import("@tauri-apps/api/path"),
    ]);

  const dir = await open({
    directory: true,
    multiple: false,
    // The dialog plugin grants the picked path into the fs scope on return.
    // `recursive: true` makes that an `allow_directory(dir, recursive)` grant
    // so writing the per-case PDFs *inside* the folder is permitted.
    recursive: true,
    title: t("cases.batch_export_pick_dir") as string,
  });
  if (typeof dir !== "string") {
    return { saved: 0, skipped: [], cancelled: true, aborted: false };
  }

  const prefix = t("cases.pdf.filename_prefix") as string;
  const used = new Set<string>();
  const skipped: string[] = [];
  let saved = 0;
  let done = 0;
  const total = caseIds.length;

  for (const id of caseIds) {
    if (signal?.aborted) {
      return { saved, skipped, cancelled: false, aborted: true, dir };
    }
    try {
      const detail = await ipc.showCase(workspaceId, id);
      if (!detail?.verdict) {
        skipped.push(id);
      } else {
        const blob = await pdf(
          <CaseVerdictPDF
            detail={detail}
            t={t}
            locale={locale}
            options={options}
          />,
        ).toBlob();
        const name = uniqueName(buildPdfFilename(detail, prefix), used);
        const path = await join(dir, name);
        const bytes = new Uint8Array(await blob.arrayBuffer());
        await writeFile(path, bytes);
        saved += 1;
      }
    } catch {
      // One bad case shouldn't sink the whole batch — record it as skipped
      // and keep going so the user still gets every other PDF.
      skipped.push(id);
    } finally {
      done += 1;
      onProgress?.({ done, total });
    }
  }

  return { saved, skipped, cancelled: false, aborted: false, dir };
}
