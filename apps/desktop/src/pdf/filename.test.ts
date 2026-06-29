// Characterization tests: they pin buildPdfFilename's CURRENT behaviour
// (sanitisation, fallbacks, date formatting) so the Cases refactor can't
// silently change the filenames clinicians already archive by.
import { describe, expect, it } from "vitest";

import type { CaseDetail } from "../lib/ipc";
import { buildPdfFilename } from "./filename";

function detailWith(over: {
  patient_label?: string;
  id?: string;
  case_date?: string;
  created_at?: string;
}): CaseDetail {
  return {
    case: {
      patient_label: over.patient_label ?? "García López",
      id: over.id ?? "case-0123456789abcdef",
      case_date: over.case_date ?? "2026-03-12T10:00:00Z",
      created_at: over.created_at ?? "2026-01-01T00:00:00Z",
    },
  } as CaseDetail;
}

describe("buildPdfFilename", () => {
  it("builds <prefix>_<label>_<YYYY-MM-DD>.pdf, spaces as underscores", () => {
    expect(buildPdfFilename(detailWith({}))).toBe(
      "ConclaveMD_García_López_2026-03-12.pdf",
    );
  });

  it("replaces filesystem-reserved characters with spaces", () => {
    expect(
      buildPdfFilename(detailWith({ patient_label: 'a/b:c*d?e"f<g>h|i' })),
    ).toBe("ConclaveMD_a_b_c_d_e_f_g_h_i_2026-03-12.pdf");
  });

  it("falls back to the first 8 chars of the case id when the label is empty", () => {
    expect(buildPdfFilename(detailWith({ patient_label: "" }))).toBe(
      "ConclaveMD_case-012_2026-03-12.pdf",
    );
  });

  it("uses created_at when case_date is empty, and today when both are junk", () => {
    expect(
      buildPdfFilename(detailWith({ case_date: "" })),
    ).toBe("ConclaveMD_García_López_2026-01-01.pdf");
    const today = new Date().toISOString().slice(0, 10);
    expect(
      buildPdfFilename(detailWith({ case_date: "not-a-date" })),
    ).toBe(`ConclaveMD_García_López_${today}.pdf`);
  });

  it("honours a custom prefix and truncates labels to 60 chars", () => {
    const long = "x".repeat(80);
    const out = buildPdfFilename(detailWith({ patient_label: long }), "Informe");
    expect(out).toBe(`Informe_${"x".repeat(60)}_2026-03-12.pdf`);
  });
});
