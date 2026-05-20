// Human-readable metadata for every backend `ProviderInfo.id`.
// The Rust side only returns stable ids; UI labels live here so the same
// strings can be reused from Settings and Cases without drift.

export type ProviderId =
  | "anthropic"
  | "openai"
  | "openrouter"
  | "ollama"
  | "anthropic-oauth"
  | "openai-oauth"
  | "apple-intelligence"
  | "claude-cli"
  | "codex-cli";

// Where a provider can be plugged in. Mirrors the Rust-side
// `ProviderScope` enum in `crates/providers/src/types.rs`.
//
// • `general` — eligible for every flow, including clinical
//   deliberation (Cases, Q&A, batch).
// • `subtask` — restricted to non-clinical utility surfaces. Hidden
//   from the deliberation pickers; the backend also refuses subtask
//   ids in those commands as a belt-and-braces.
export type ProviderScope = "general" | "subtask";

export type ProviderMeta = {
  id: ProviderId;
  name: string;
  tagline: string;
  authLabel: "OAuth" | "API key" | "Local" | "Local CLI";
  monogram: string;
  // Tailwind color stem (e.g. "amber") used for the monogram tint and hover ring.
  brand: "amber" | "emerald" | "sky" | "violet" | "slate";
  // `true` when the provider is available without a credential and should not
  // count against the one-active-provider rule. Ollama and Apple Intelligence
  // qualify today.
  alwaysAvailable?: boolean;
  recommended?: boolean;
  // Defaults to "general" when omitted — keeps existing entries terse.
  scope?: ProviderScope;
};

export const PROVIDER_META: Record<string, ProviderMeta> = {
  "claude-cli": {
    id: "claude-cli",
    name: "Claude Code (local)",
    tagline: "Tu CLI oficial · tu suscripción",
    authLabel: "Local CLI",
    monogram: "C",
    brand: "amber",
  },
  "codex-cli": {
    id: "codex-cli",
    name: "Codex (local)",
    tagline: "Tu CLI oficial · tu suscripción",
    authLabel: "Local CLI",
    monogram: "G",
    brand: "emerald",
  },
  "anthropic-oauth": {
    id: "anthropic-oauth",
    name: "Claude Max",
    tagline: "Suscripción Anthropic",
    authLabel: "OAuth",
    monogram: "C",
    brand: "amber",
  },
  "openai-oauth": {
    id: "openai-oauth",
    name: "ChatGPT",
    tagline: "Suscripción OpenAI",
    authLabel: "OAuth",
    monogram: "G",
    brand: "emerald",
  },
  anthropic: {
    id: "anthropic",
    name: "Anthropic API",
    tagline: "Clave de developer",
    authLabel: "API key",
    monogram: "A",
    brand: "amber",
    recommended: true,
  },
  openai: {
    id: "openai",
    name: "OpenAI API",
    tagline: "Clave de developer",
    authLabel: "API key",
    monogram: "O",
    brand: "emerald",
  },
  openrouter: {
    id: "openrouter",
    name: "OpenRouter",
    tagline: "Pasarela multi-modelo",
    authLabel: "API key",
    monogram: "R",
    brand: "violet",
  },
  ollama: {
    id: "ollama",
    name: "Ollama",
    tagline: "Modelos locales en tu Mac",
    authLabel: "Local",
    monogram: "·",
    brand: "slate",
    alwaysAvailable: true,
  },
  "apple-intelligence": {
    id: "apple-intelligence",
    name: "Apple Intelligence",
    tagline: "On-device · solo subtareas",
    authLabel: "Local",
    monogram: "",
    brand: "sky",
    alwaysAvailable: true,
    scope: "subtask",
  },
};

export function metaFor(id: string): ProviderMeta {
  return (
    PROVIDER_META[id] ?? {
      id: id as ProviderId,
      name: id,
      tagline: "",
      authLabel: "API key",
      monogram: id.slice(0, 1).toUpperCase(),
      brand: "slate",
    }
  );
}

export function isOccupyingSlot(id: string): boolean {
  return !PROVIDER_META[id]?.alwaysAvailable;
}

// Eligible for clinical deliberation (Cases, Q&A, batch). Subtask-only
// providers (e.g. Apple Intelligence) are filtered out of those pickers
// because their vendor guardrails reject clinical content.
export function isClinicalEligible(id: string): boolean {
  return (PROVIDER_META[id]?.scope ?? "general") !== "subtask";
}

// Pick the best default provider id from a candidate list. Callers
// must filter for clinical-eligibility first when they need that.
//
// Preference order:
//   1. `configured && available` — the user has actively connected this
//      provider AND its backend is reachable right now. This is what
//      makes a signed-in OpenAI OAuth account win over an offline
//      Ollama instance.
//   2. `available` — anything reachable (e.g. Ollama running without
//      auth, the developer-mode local-only case).
//   3. The first entry in the list — pure fallback so the form always
//      has *some* selection; the user will hit a clean error from
//      `ensure_provider_ready` when they try to run.
export function preferredProvider(
  providers: { id: string; configured?: boolean; available?: boolean }[],
): string | null {
  const ready = providers.find((p) => p.configured && p.available);
  if (ready) return ready.id;
  const avail = providers.find((p) => p.available);
  if (avail) return avail.id;
  return providers[0]?.id ?? null;
}

// Tailwind class fragments per brand. Inline (rather than computed) so the
// JIT picks them up without a safelist.
export const BRAND_TINT: Record<ProviderMeta["brand"], string> = {
  amber: "bg-amber-400/12 text-amber-200 ring-amber-400/30",
  emerald: "bg-emerald-400/12 text-emerald-200 ring-emerald-400/30",
  sky: "bg-sky-400/12 text-sky-200 ring-sky-400/30",
  violet: "bg-violet-400/12 text-violet-200 ring-violet-400/30",
  slate: "bg-slate-400/12 text-slate-200 ring-slate-400/30",
};

export const BRAND_HOVER: Record<ProviderMeta["brand"], string> = {
  amber: "hover:border-amber-400/40",
  emerald: "hover:border-emerald-400/40",
  sky: "hover:border-sky-400/40",
  violet: "hover:border-violet-400/40",
  slate: "hover:border-slate-400/40",
};

// Provider groups for the Settings picker, ordered by **compliance and
// stability**:
//
//   1. **Local CLI** (`claude-cli`, `codex-cli`) when at least one of
//      the binaries is detected on `$PATH` AND the user is signed in
//      via the CLI's own login flow. This is the only vendor-sanctioned
//      way to use a Pro/Max/Plus subscription with Conclave: the
//      official CLI binary makes the request under the user's own
//      account.
//   2. **API key** (`anthropic`, `openai`, `openrouter`) — the
//      contractually sanctioned developer path, paid per-use.
//   3. **Subscriptions / OAuth** (`anthropic-oauth`, `openai-oauth`) —
//      reuses the Claude Code / Codex CLI client_id from inside
//      Conclave. Not vendor-supported; rendered last with an
//      "unofficial" disclaimer.
//
// When neither CLI binary is detected, the Local CLI group still
// appears (so the user knows the option exists and how to enable it),
// but in second position with install hints instead of clickable
// tiles, and the "Recommended" chip stays on the API key group.
//
// On-device providers (Ollama, Apple Intelligence) are *not* listed
// here: they're surfaced through dedicated notes rendered alongside
// the picker and active-provider views, because they don't have a
// "connect" step the user has to walk through.

export type PickerGroup = {
  titleKey: string;
  captionKey?: string;
  ids: ProviderId[];
};

type GroupInputProvider = {
  id: string;
  configured?: boolean;
  available?: boolean;
};

/**
 * Resolve the order of the picker groups (and which copy variant to
 * use for the Local CLI caption) given the current backend
 * `ProviderInfo` list.
 */
export function buildPickerGroups(providers: GroupInputProvider[]): PickerGroup[] {
  const cliAvailable = providers.some(
    (p) => isLocalCli(p.id) && p.configured && p.available,
  );
  const cliGroup: PickerGroup = {
    titleKey: "settings.picker_group_cli",
    captionKey: cliAvailable
      ? "settings.picker_group_cli_caption"
      : "settings.picker_group_cli_caption_not_installed",
    ids: ["claude-cli", "codex-cli"],
  };
  const apiGroup: PickerGroup = {
    titleKey: "settings.picker_group_api",
    captionKey: "settings.picker_group_api_caption",
    ids: ["anthropic", "openai", "openrouter"],
  };
  const oauthGroup: PickerGroup = {
    titleKey: "settings.picker_group_oauth",
    captionKey: "settings.picker_group_oauth_caption",
    ids: ["anthropic-oauth", "openai-oauth"],
  };
  return cliAvailable
    ? [cliGroup, apiGroup, oauthGroup]
    : [apiGroup, cliGroup, oauthGroup];
}

/**
 * `true` when the picker should pin the "Recomendado" chip to the
 * first signed-in CLI provider rather than to the API key tile. We
 * recompute this each render because the user can sign into the CLI
 * (or sign out) outside of Conclave during the session.
 */
export function shouldRecommendCli(providers: GroupInputProvider[]): boolean {
  return providers.some((p) => isLocalCli(p.id) && p.configured && p.available);
}

// Convenience: which provider ids use the (unofficial) subscription
// OAuth path. The UI uses this to render disclaimer copy and route
// auth-failure messaging.
export function isSubscriptionOAuth(id: string): boolean {
  return id === "anthropic-oauth" || id === "openai-oauth";
}

/**
 * `true` for the local-CLI proxy providers (vendor-sanctioned path:
 * Conclave shells out to the user's own `claude` / `codex` binary
 * with their own credentials).
 */
export function isLocalCli(id: string): boolean {
  return id === "claude-cli" || id === "codex-cli";
}
