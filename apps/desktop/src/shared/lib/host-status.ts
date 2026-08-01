/**
 * Product-facing host presence for the desktop shell (IM-aligned).
 *
 * Planes (do not collapse):
 * - Runtime (A): local daemon IPC usable → Ready / Unavailable / Preview
 * - Link (B): account Host Link binding → Local only / Linked
 * - Hub (D): this Mac's live `/ws/host` on the server → Hub online / offline
 * - Project locus (C): which machine owns the project → This Mac / device name
 *
 * Local coding is first-class: missing hub is not "daemon offline".
 * Mobile "device online" is Hub (D), not Link (B).
 */

export type DataSource = "mock" | "daemon";

/** Collaboration link to backend / other hosts (plane B). */
export type HostLinkMode = "local_only" | "linked";

/** Live hub session for this host installation (plane D). */
export type HubOnlineMode = "online" | "offline" | "unknown";

/** App-level readiness shown under the brand mark (planes A + B + D). */
export type HostPresenceTone = "ready" | "unavailable" | "preview";

export type HostPresence = {
  /** Dot + primary line tone. */
  tone: HostPresenceTone;
  /**
   * Primary label under Minos, e.g. "Ready · Linked · Hub online".
   * Never exposes managed / discovery implementation detail.
   */
  label: string;
  /** Short secondary phrase for Host page cards. */
  readinessLabel: "Ready" | "Unavailable" | "Preview";
  linkMode: HostLinkMode;
  linkLabel: "Local only" | "Linked";
  hubOnline: HubOnlineMode;
  hubLabel: "Hub online" | "Hub offline" | "Hub unknown";
  /** True when local coding runtime is usable. */
  runtimeReady: boolean;
};

export type HostPresenceInput = {
  source: DataSource;
  /** True when Tauri bridge reports daemon connected. */
  daemonConnected: boolean;
  /**
   * Host Link binding for remote collaboration (account ↔ this Mac).
   * `true` → Linked; `false` / omit → Local only when runtime is ready.
   */
  relayLinked?: boolean;
  /**
   * Live `/ws/host` to minos-backend (IM device online).
   * Omit → unknown (external daemon without link observer).
   */
  hubOnline?: boolean;
};

/** Default project locus for projects on this machine (plane C). */
export const PROJECT_HOST_THIS_MAC = "This Mac";

export function deriveHostPresence(input: HostPresenceInput): HostPresence {
  if (input.source === "mock") {
    return {
      tone: "preview",
      label: "Preview",
      readinessLabel: "Preview",
      linkMode: "local_only",
      linkLabel: "Local only",
      hubOnline: "unknown",
      hubLabel: "Hub unknown",
      runtimeReady: false,
    };
  }

  if (!input.daemonConnected) {
    return {
      tone: "unavailable",
      label: "Unavailable",
      readinessLabel: "Unavailable",
      linkMode: "local_only",
      linkLabel: "Local only",
      hubOnline: "offline",
      hubLabel: "Hub offline",
      runtimeReady: false,
    };
  }

  const linked = input.relayLinked === true;
  const linkMode: HostLinkMode = linked ? "linked" : "local_only";
  const linkLabel = linked ? "Linked" : "Local only";

  let hubOnline: HubOnlineMode = "unknown";
  let hubLabel: HostPresence["hubLabel"] = "Hub unknown";
  if (input.hubOnline === true) {
    hubOnline = "online";
    hubLabel = "Hub online";
  } else if (input.hubOnline === false) {
    hubOnline = "offline";
    hubLabel = "Hub offline";
  }

  // When linked, surface hub state so Mobile device-online is understandable.
  const label =
    linked && hubOnline !== "unknown"
      ? `Ready · ${linkLabel} · ${hubLabel}`
      : `Ready · ${linkLabel}`;

  return {
    tone: "ready",
    label,
    readinessLabel: "Ready",
    linkMode,
    linkLabel,
    hubOnline,
    hubLabel,
    runtimeReady: true,
  };
}

/**
 * Host chip on the project header / list.
 * v1: always this Mac; later pass `hostName` from multi-device project rows.
 */
export function projectHostLabel(hostName?: string | null): string {
  const trimmed = hostName?.trim();
  if (trimmed) return trimmed;
  return PROJECT_HOST_THIS_MAC;
}

export function presenceDotClass(tone: HostPresenceTone): string {
  switch (tone) {
    case "ready":
      return "text-emerald-500";
    case "unavailable":
      return "text-rose-500";
    case "preview":
      return "text-amber-500";
  }
}

export function projectHostPillClass(hostLabel: string): string {
  // This Mac stays calm/neutral; remote hosts can use a stronger chip later.
  if (hostLabel === PROJECT_HOST_THIS_MAC) {
    return "bg-surface-muted text-ink-secondary ring-1 ring-ink/10";
  }
  return "bg-sky-50 text-sky-900 ring-1 ring-sky-200/80";
}
