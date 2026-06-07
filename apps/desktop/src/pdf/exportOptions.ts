// User-configurable extras for the case-verdict PDF.
//
// Every flag defaults OFF and `headerNote` defaults to "" so the baseline
// export is byte-for-byte identical to the pre-customization output — that
// keeps the document layout stable for anyone who never touches the options.
//
// Persistence is frontend-only (localStorage), mirroring `lib/theme.ts`: the
// PDF pipeline is entirely client-side, so there is nothing for the Rust side
// to store. Filenames can carry personal data, hence `includeSourceFiles`
// stays off until the clinician opts in.

export interface PdfExportOptions {
  /** Render a "Source documents" section listing each attachment's original
   *  filename. Off by default — filenames may contain patient identifiers. */
  includeSourceFiles: boolean;
  /** When the source-documents section is shown, also print each file's type
   *  and size. Has no effect unless `includeSourceFiles` is true. */
  includeAttachmentMeta: boolean;
  /** Free-text note rendered just under the header (clinic name, internal
   *  reference…). Empty string means "no note". */
  headerNote: string;
  /** Render a "Generation details" block: provider · model, token counts and
   *  latency taken from the verdict record. */
  includeGenerationMeta: boolean;
}

export const DEFAULT_PDF_EXPORT_OPTIONS: PdfExportOptions = {
  includeSourceFiles: false,
  includeAttachmentMeta: false,
  headerNote: "",
  includeGenerationMeta: false,
};

const STORAGE_KEY = "conclave.pdf-export-options";

/** Upper bound on the header note so a runaway paste can't blow up the PDF
 *  layout (or localStorage). Enforced on both load and save. */
export const MAX_HEADER_NOTE = 200;

/** Coerces arbitrary parsed JSON into a valid options object, filling any
 *  missing or wrongly-typed field from the defaults. This lets the persisted
 *  shape grow over time without older blobs breaking the parse. */
function normalise(raw: unknown): PdfExportOptions {
  if (typeof raw !== "object" || raw === null) {
    return { ...DEFAULT_PDF_EXPORT_OPTIONS };
  }
  const o = raw as Record<string, unknown>;
  const bool = (v: unknown, fallback: boolean): boolean =>
    typeof v === "boolean" ? v : fallback;
  return {
    includeSourceFiles: bool(
      o.includeSourceFiles,
      DEFAULT_PDF_EXPORT_OPTIONS.includeSourceFiles,
    ),
    includeAttachmentMeta: bool(
      o.includeAttachmentMeta,
      DEFAULT_PDF_EXPORT_OPTIONS.includeAttachmentMeta,
    ),
    headerNote:
      typeof o.headerNote === "string"
        ? o.headerNote.slice(0, MAX_HEADER_NOTE)
        : DEFAULT_PDF_EXPORT_OPTIONS.headerNote,
    includeGenerationMeta: bool(
      o.includeGenerationMeta,
      DEFAULT_PDF_EXPORT_OPTIONS.includeGenerationMeta,
    ),
  };
}

/** Reads the persisted options, always returning a complete, valid object.
 *  Falls back to defaults when storage is unavailable or the blob is junk. */
export function loadPdfExportOptions(): PdfExportOptions {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULT_PDF_EXPORT_OPTIONS };
    return normalise(JSON.parse(raw));
  } catch {
    /* localStorage unavailable or malformed JSON */
    return { ...DEFAULT_PDF_EXPORT_OPTIONS };
  }
}

/** Persists the options. Trims the header note to the max length so what we
 *  store matches what `load` would return. */
export function savePdfExportOptions(options: PdfExportOptions): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        ...options,
        headerNote: options.headerNote.slice(0, MAX_HEADER_NOTE),
      }),
    );
  } catch {
    /* localStorage unavailable */
  }
}
