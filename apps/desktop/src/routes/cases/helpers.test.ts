// Characterization tests written as part of extracting these helpers out
// of Cases.tsx — they pin current behaviour so the decomposition can't
// drift it.
import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";

import {
  attachmentFromPath,
  bucketAnchor,
  bucketKey,
  dedupeAttachments,
  formatBytes,
  formatElapsed,
  isFallbackLabel,
  isoToLocalInput,
  localInputToIso,
} from "./helpers";
import { parseFailedPhase, stripCodeFences, tryParseVerdict } from "./verdictParsing";

const tKey = ((key: string) => key) as unknown as TFunction;
void tKey; // reserved for bucketLabel tests below if locale-stable cases are added

describe("attachmentFromPath", () => {
  it("accepts supported extensions and flags images", () => {
    expect(attachmentFromPath("/a/b/informe.PDF")).toEqual({
      path: "/a/b/informe.PDF",
      name: "informe.PDF",
      ext: "pdf",
      isImage: false,
    });
    expect(attachmentFromPath("C:\\scans\\torax.jpeg")?.isImage).toBe(true);
  });

  it("rejects unsupported or extension-less paths", () => {
    expect(attachmentFromPath("/a/b/script.exe")).toBeNull();
    expect(attachmentFromPath("/a/b/README")).toBeNull();
  });
});

describe("dedupeAttachments", () => {
  it("keeps base order and drops duplicate paths from incoming", () => {
    const a = attachmentFromPath("/x/a.pdf")!;
    const b = attachmentFromPath("/x/b.pdf")!;
    expect(dedupeAttachments([a], [a, b]).map((x) => x.path)).toEqual([
      "/x/a.pdf",
      "/x/b.pdf",
    ]);
  });
});

describe("formatBytes / formatElapsed", () => {
  it("formats byte sizes in B/KB/MB", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });

  it("formats durations compactly across the hour boundary", () => {
    expect(formatElapsed(42_000)).toBe("0:42");
    expect(formatElapsed(727_000)).toBe("12:07");
    expect(formatElapsed(3_780_000)).toBe("1h 03m");
    expect(formatElapsed(-5)).toBe("0:00");
    expect(formatElapsed(Number.NaN)).toBe("0:00");
  });
});

describe("isFallbackLabel", () => {
  it("treats empty, space-less and underscored labels as fallbacks", () => {
    expect(isFallbackLabel(null)).toBe(true);
    expect(isFallbackLabel("CR-IA-007")).toBe(true);
    expect(isFallbackLabel("case_recto_bajo")).toBe(true);
    expect(isFallbackLabel("Mujer 67, recto bajo T3N1")).toBe(false);
  });
});

describe("bucket helpers", () => {
  it("anchors weeks on Monday (Sunday belongs to the previous Monday)", () => {
    // 2026-06-07 is a Sunday → its week anchor is Monday 2026-06-01.
    const anchor = bucketAnchor("2026-06-07T12:00:00", "week");
    expect(anchor.getDay()).toBe(1);
    expect(anchor.getDate()).toBe(1);
  });

  it("anchors months on day 1 and builds stable keys", () => {
    expect(bucketKey("2026-06-11T09:30:00", "month")).toBe("2026-6");
    expect(bucketKey("2026-06-11T09:30:00", "day")).toBe("2026-6-11");
    expect(bucketKey("whatever", "off")).toBe("all");
  });
});

describe("datetime-local conversion", () => {
  it("round-trips a local wall-clock time through ISO", () => {
    const localInput = "2026-06-11T09:30";
    const iso = localInputToIso(localInput);
    expect(isoToLocalInput(iso)).toBe(localInput);
  });
});

describe("verdict parsing", () => {
  it("strips ```json fences and plain fences", () => {
    expect(stripCodeFences('```json\n{"a":1}\n```')).toBe('{"a":1}');
    expect(stripCodeFences("```\nhola\n```")).toBe("hola");
    expect(stripCodeFences("  raw  ")).toBe("raw");
  });

  it("extracts the failed deliberation phase from persisted errors", () => {
    expect(
      parseFailedPhase("deliberation phase redteam failed: provider error"),
    ).toBe("redteam");
    expect(parseFailedPhase("some legacy error")).toBeNull();
  });

  it("parses a structurally complete verdict and fills optional fields", () => {
    const verdict = tryParseVerdict(
      JSON.stringify({
        case_summary: "s",
        key_clinical_data: [],
        applied_evidence: [],
        primary_recommendation: { action: "a", rationale: "r" },
        certainty_level: "medium",
        red_flags: [],
        follow_up_triggers: [],
      }),
    );
    expect(verdict?.primary_recommendation.action).toBe("a");
    expect(verdict?.certainty_justification).toBe("");
    expect(verdict?.disclaimer).toBe("");
  });

  it("returns null on structural mismatch or invalid JSON", () => {
    expect(tryParseVerdict("{nope")).toBeNull();
    expect(tryParseVerdict(JSON.stringify({ case_summary: "s" }))).toBeNull();
  });
});
