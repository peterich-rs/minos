/**
 * Product-facing host presence for the desktop shell.
 *
 * Three planes (do not collapse into "Daemon · managed"):
 * - Runtime (A): local daemon usable → Ready / Unavailable / Preview
 * - Link (B): relay/backend collaboration → Local only / Linked
 * - Project locus (C): which machine owns the project → This Mac / device name
 *
 * Local coding is first-class: missing relay is "Local only", not Offline.
 */

export type DataSource = "mock" | "daemon";

/** Collaboration link to backend / other hosts (plane B). */
export type HostLinkMode = "local_only" | "linked";

/** App-level readiness shown under the brand mark (planes A + B). */
export type HostPresenceTone = "ready" | "unavailable" | "preview";

export type HostPresence = {
  /** Dot + primary line tone. */
  tone: HostPresenceTone;
  /**
   * Primary label under Minos, e.g. "Ready · Local only".
   * Never exposes managed / discovery implementation detail.
   */
  label: string;
  /** Short secondary phrase for Host page cards. */
  readinessLabel: "Ready" | "Unavailable" | "Preview";
  linkMode: HostLinkMode;
  linkLabel: "Local only" | "Linked";
  /** True when local coding runtime is usable. */
  runtimeReady: boolean;
};

export type HostPresenceInput = {
  source: DataSource;
  /** True when Tauri bridge reports daemon connected. */
  daemonConnected: boolean;
  /**
   * Relay / backend linked for remote collaboration.
   * `true` → Linked; `false` / omit → Local only when runtime is ready.
   * Desktop: driven by account-store Host Link state after same-account link.
   */
  relayLinked?: boolean;
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
      runtimeReady: false,
    };
  }

  const linked = input.relayLinked === true;
  const linkMode: HostLinkMode = linked ? "linked" : "local_only";
  const linkLabel = linked ? "Linked" : "Local only";

  return {
    tone: "ready",
    label: `Ready · ${linkLabel}`,
    readinessLabel: "Ready",
    linkMode,
    linkLabel,
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
