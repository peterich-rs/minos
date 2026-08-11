/**
 * Product-facing presence for the desktop shell (ADR 0021 / participant delivery).
 *
 * User model:
 * - Local runtime: Ready / Unavailable / Preview
 * - **Primary Online** = Account IM sync (`/ws/client` live → can send/receive)
 * - Host readiness (`/ws/host`) = secondary "This Mac ready" for bot runtime
 *
 * Never show full Online solely because Host is live while Account cannot sync.
 */

export type DataSource = "mock" | "daemon";

/** Product cloud/IM mode for Account-primary Online. */
export type CloudMode = "online" | "connecting" | "offline" | "unknown";

/** App-level readiness shown under the brand mark. */
export type HostPresenceTone = "ready" | "unavailable" | "preview" | "connecting";

export type HostPresence = {
  tone: HostPresenceTone;
  /**
   * Primary label under Minos, e.g. "Online", "Connecting…", "Offline".
   * Reflects Account IM ability to send/receive — not Host alone.
   */
  label: string;
  readinessLabel: "Ready" | "Unavailable" | "Preview";
  cloud: CloudMode;
  cloudLabel: "Online" | "Connecting" | "Offline" | "—";
  /** True when local coding runtime is usable. */
  runtimeReady: boolean;
  /** Secondary: Host `/ws/host` live (bot runtime). */
  hostReady: boolean;
  hostLabel: "Host ready" | "Host offline" | "—";
};

export type HostPresenceInput = {
  source: DataSource;
  /** True when Tauri bridge reports daemon connected. */
  daemonConnected: boolean;
  /**
   * Account IM sync status (primary Online). Prefer this over host-only signals.
   * Maps from `/ws/client` CloudRealtimeSyncState via account-store.
   */
  accountSync?: CloudMode;
  /**
   * @deprecated Host-centric cloud status. Used only when `accountSync` omitted
   * (legacy callers). New UI should pass `accountSync`.
   */
  cloud?: CloudMode;
  /**
   * Live `/ws/host` for bot runtime readiness (secondary).
   */
  cloudOnline?: boolean;
};

/** Default project locus for projects on this machine. */
export const PROJECT_HOST_THIS_MAC = "This Mac";

export function deriveHostPresence(input: HostPresenceInput): HostPresence {
  const hostReady = input.cloudOnline === true;
  const hostLabel: HostPresence["hostLabel"] =
    input.cloudOnline === true
      ? "Host ready"
      : input.cloudOnline === false
        ? "Host offline"
        : "—";

  if (input.source === "mock") {
    return {
      tone: "preview",
      label: "Preview",
      readinessLabel: "Preview",
      cloud: "unknown",
      cloudLabel: "—",
      runtimeReady: false,
      hostReady: false,
      hostLabel: "—",
    };
  }

  if (!input.daemonConnected) {
    return {
      tone: "unavailable",
      label: "Unavailable",
      readinessLabel: "Unavailable",
      cloud: "offline",
      cloudLabel: "Offline",
      runtimeReady: false,
      hostReady: false,
      hostLabel: "Host offline",
    };
  }

  // Primary Online = Account IM sync. Fall back to legacy `cloud` only when
  // accountSync is not provided (unit tests / old call sites).
  const cloud: CloudMode =
    input.accountSync ??
    input.cloud ??
    (input.cloudOnline === true
      ? "online"
      : input.cloudOnline === false
        ? "offline"
        : "unknown");

  if (cloud === "connecting") {
    return {
      tone: "connecting",
      label: "Connecting…",
      readinessLabel: "Ready",
      cloud: "connecting",
      cloudLabel: "Connecting",
      runtimeReady: true,
      hostReady,
      hostLabel,
    };
  }

  if (cloud === "online") {
    return {
      tone: "ready",
      label: "Online",
      readinessLabel: "Ready",
      cloud: "online",
      cloudLabel: "Online",
      runtimeReady: true,
      hostReady,
      hostLabel,
    };
  }

  if (cloud === "offline") {
    return {
      tone: "ready",
      label: "Offline",
      readinessLabel: "Ready",
      cloud: "offline",
      cloudLabel: "Offline",
      runtimeReady: true,
      hostReady,
      hostLabel,
    };
  }

  return {
    tone: "ready",
    label: "Ready",
    readinessLabel: "Ready",
    cloud: "unknown",
    cloudLabel: "—",
    runtimeReady: true,
    hostReady,
    hostLabel,
  };
}

/**
 * Map Hub `/ws/client` realtime state → product CloudMode (Account IM).
 * live/syncing → online (can send/receive); connecting → connecting; else offline.
 */
export function cloudModeFromAccountSync(
  state: "disconnected" | "connecting" | "syncing" | "live" | "error" | string,
): CloudMode {
  switch (state) {
    case "live":
    case "syncing":
      return "online";
    case "connecting":
      return "connecting";
    case "disconnected":
    case "error":
      return "offline";
    default:
      return "unknown";
  }
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
    case "connecting":
      return "text-amber-500";
  }
}

export function projectHostPillClass(hostLabel: string): string {
  if (hostLabel === PROJECT_HOST_THIS_MAC) {
    return "bg-surface-muted text-ink-secondary ring-1 ring-ink/10";
  }
  return "bg-sky-50 text-sky-900 ring-1 ring-sky-200/80";
}
