// Characterization tests for the provider metadata helpers the Settings
// and Cases pickers branch on. Pure module — no Tauri imports.
import { describe, expect, it } from "vitest";

import {
  PROVIDER_META,
  buildPickerGroups,
  isClinicalEligible,
  isLocalCli,
  isOccupyingSlot,
  isSubscriptionOAuth,
  metaFor,
  preferredProvider,
  shouldRecommendCli,
} from "./providers";

describe("metaFor", () => {
  it("returns the registered metadata for known ids", () => {
    expect(metaFor("anthropic")).toBe(PROVIDER_META.anthropic);
  });

  it("synthesises a slate fallback for unknown ids", () => {
    const meta = metaFor("mystery-llm");
    expect(meta.name).toBe("mystery-llm");
    expect(meta.monogram).toBe("M");
    expect(meta.brand).toBe("slate");
    expect(meta.authLabel).toBe("API key");
  });
});

describe("slot and eligibility rules", () => {
  it("always-available locals don't occupy the provider slot", () => {
    expect(isOccupyingSlot("ollama")).toBe(false);
    expect(isOccupyingSlot("apple-intelligence")).toBe(false);
    expect(isOccupyingSlot("anthropic")).toBe(true);
  });

  it("subtask-scoped providers are excluded from clinical flows", () => {
    expect(isClinicalEligible("apple-intelligence")).toBe(false);
    expect(isClinicalEligible("anthropic")).toBe(true);
    expect(isClinicalEligible("unknown-id")).toBe(true);
  });

  it("classifies the CLI and OAuth pairs", () => {
    expect(isLocalCli("claude-cli")).toBe(true);
    expect(isLocalCli("anthropic")).toBe(false);
    expect(isSubscriptionOAuth("openai-oauth")).toBe(true);
    expect(isSubscriptionOAuth("openai")).toBe(false);
  });
});

describe("preferredProvider", () => {
  it("prefers the first ready provider over list order", () => {
    expect(
      preferredProvider([
        { id: "ollama", status: "login_required" },
        { id: "claude-cli", status: "ready" },
      ]),
    ).toBe("claude-cli");
  });

  it("falls back to the first entry, and null on empty", () => {
    expect(preferredProvider([{ id: "openai" }])).toBe("openai");
    expect(preferredProvider([])).toBeNull();
  });
});

describe("buildPickerGroups / shouldRecommendCli", () => {
  const cliReady = [{ id: "claude-cli", status: "ready" }];
  const cliMissing = [{ id: "anthropic", status: "ready" }];

  it("puts the CLI group first (with the signed-in caption) when a CLI is ready", () => {
    const groups = buildPickerGroups(cliReady);
    expect(groups.map((g) => g.titleKey)).toEqual([
      "settings.picker_group_cli",
      "settings.picker_group_api",
      "settings.picker_group_oauth",
    ]);
    expect(groups[0].captionKey).toBe("settings.picker_group_cli_caption");
    expect(shouldRecommendCli(cliReady)).toBe(true);
  });

  it("leads with the API group (CLI gets install hints) when no CLI is ready", () => {
    const groups = buildPickerGroups(cliMissing);
    expect(groups.map((g) => g.titleKey)).toEqual([
      "settings.picker_group_api",
      "settings.picker_group_cli",
      "settings.picker_group_oauth",
    ]);
    expect(groups[1].captionKey).toBe(
      "settings.picker_group_cli_caption_not_installed",
    );
    expect(shouldRecommendCli(cliMissing)).toBe(false);
  });
});
