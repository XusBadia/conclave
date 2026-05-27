// Single source of truth for the per-provider state-machine derived
// from the backend's `ProviderInfo.status`. Lives in its own module so
// the UI components and the ipc-layer call-site filters agree on what
// "usable" / "configured" mean.
//
// The previous shape (`configured` + `available`) let those two
// signals drift apart, which produced the contradictory "Connected ·
// Reachable · authentication failed" UI we're fixing here.

import type { ProviderInfo, ProviderStatus } from "./ipc";

/** `true` when the provider can actually be called right now. */
export function isReady(p: ProviderInfo): boolean {
  return p.status === "ready";
}

/**
 * `true` when the user has connected the provider in some form —
 * credential present, CLI logged in, etc. Mirrors the old
 * `configured` boolean: includes states where the credential is
 * present but the upstream is unhappy (`expired`, `unreachable`).
 *
 * Used by Settings to decide whether to show the active-provider
 * card vs the picker, and by the slot-migration dialog.
 */
export function isConfigured(p: ProviderInfo): boolean {
  return p.status !== "not_configured" && p.status !== "not_installed";
}

/**
 * Color stem applied to the `<ProviderStatusPill>` background and
 * text. Centralised here so any other surface that needs to tint by
 * status (e.g. the cases-page strip) can match without duplicating
 * the mapping.
 */
export function statusTone(
  s: ProviderStatus,
): "ok" | "warn" | "neutral" {
  switch (s) {
    case "ready":
      return "ok";
    case "expired":
    case "unreachable":
    case "login_required":
      return "warn";
    case "not_configured":
    case "not_installed":
      return "neutral";
  }
}

/** i18n key for the short pill label. */
export function statusLabelKey(s: ProviderStatus): string {
  switch (s) {
    case "ready":
      return "settings.status_ready";
    case "expired":
      return "settings.status_expired";
    case "unreachable":
      return "settings.status_unreachable";
    case "not_configured":
      return "settings.status_not_configured";
    case "login_required":
      return "settings.status_login_required";
    case "not_installed":
      return "settings.status_not_installed";
  }
}
