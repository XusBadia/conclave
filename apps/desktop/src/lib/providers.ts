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
  | "apple-intelligence";

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
  authLabel: "OAuth" | "API key" | "Local";
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

// Display order in the picker grid. API keys first because that's the
// contractually sanctioned path — stable, fully supported by the
// vendors, and unaffected by enforcement actions against unofficial
// CLI-OAuth reuse. Subscription OAuth is offered as a secondary
// convenience with an explicit "may be revoked" disclaimer in the UI.
// Group titles and captions are expressed as i18n keys so the UI
// stays locale-aware.
//
// On-device providers (Ollama, Apple Intelligence) are *not* listed here:
// they're surfaced through dedicated notes rendered alongside the picker and
// active-provider views, because they don't have a "connect" step the user
// has to walk through.
export const PICKER_GROUPS: {
  titleKey: string;
  captionKey?: string;
  ids: ProviderId[];
}[] = [
  {
    titleKey: "settings.picker_group_api",
    captionKey: "settings.picker_group_api_caption",
    ids: ["anthropic", "openai", "openrouter"],
  },
  {
    titleKey: "settings.picker_group_oauth",
    captionKey: "settings.picker_group_oauth_caption",
    ids: ["anthropic-oauth", "openai-oauth"],
  },
];

// Convenience: which provider ids use the (unofficial) subscription
// OAuth path. The UI uses this to render disclaimer copy and route
// auth-failure messaging.
export function isSubscriptionOAuth(id: string): boolean {
  return id === "anthropic-oauth" || id === "openai-oauth";
}
