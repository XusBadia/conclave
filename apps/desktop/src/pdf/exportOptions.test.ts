// Characterization tests for the persisted PDF-export options: defaults
// stay byte-stable, junk in localStorage never breaks the load path, and
// the header note is capped on both load and save.
import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_PDF_EXPORT_OPTIONS,
  MAX_HEADER_NOTE,
  loadPdfExportOptions,
  savePdfExportOptions,
} from "./exportOptions";

const STORAGE_KEY = "conclave.pdf-export-options";

beforeEach(() => {
  localStorage.clear();
});

describe("DEFAULT_PDF_EXPORT_OPTIONS", () => {
  it("keeps every flag off and the note empty (baseline PDF unchanged)", () => {
    expect(DEFAULT_PDF_EXPORT_OPTIONS).toEqual({
      includeSourceFiles: false,
      includeAttachmentMeta: false,
      headerNote: "",
      includeGenerationMeta: false,
    });
  });
});

describe("loadPdfExportOptions", () => {
  it("returns a fresh copy of the defaults when storage is empty", () => {
    const loaded = loadPdfExportOptions();
    expect(loaded).toEqual(DEFAULT_PDF_EXPORT_OPTIONS);
    expect(loaded).not.toBe(DEFAULT_PDF_EXPORT_OPTIONS);
  });

  it("survives corrupted JSON by falling back to defaults", () => {
    localStorage.setItem(STORAGE_KEY, "{nope");
    expect(loadPdfExportOptions()).toEqual(DEFAULT_PDF_EXPORT_OPTIONS);
  });

  it("fills missing or wrongly-typed fields from the defaults", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ includeSourceFiles: true, headerNote: 42 }),
    );
    expect(loadPdfExportOptions()).toEqual({
      ...DEFAULT_PDF_EXPORT_OPTIONS,
      includeSourceFiles: true,
    });
  });

  it("caps a stored header note at MAX_HEADER_NOTE on load", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ headerNote: "n".repeat(MAX_HEADER_NOTE + 50) }),
    );
    expect(loadPdfExportOptions().headerNote).toHaveLength(MAX_HEADER_NOTE);
  });
});

describe("savePdfExportOptions", () => {
  it("round-trips through load and trims the note on save", () => {
    savePdfExportOptions({
      ...DEFAULT_PDF_EXPORT_OPTIONS,
      includeGenerationMeta: true,
      headerNote: "h".repeat(MAX_HEADER_NOTE + 10),
    });
    const loaded = loadPdfExportOptions();
    expect(loaded.includeGenerationMeta).toBe(true);
    expect(loaded.headerNote).toBe("h".repeat(MAX_HEADER_NOTE));
  });
});
