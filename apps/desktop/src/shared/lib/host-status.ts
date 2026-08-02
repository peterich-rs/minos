/**
 * Product-facing host / cloud presence for the desktop shell.
 *
 * User model (simple):
 * - Local runtime: Ready / Unavailable / Preview
 * - Cloud: Online / Connecting / Offline (login implies host control; no "Link")
 *
 * Mobile "device online" === cloud Online (live `/ws/host`).
 */

export type DataSource = "mock" | "daemon";

/** Live hub session for this host installation. */
export type CloudMode = "online" | "connecting" | "offline" | "unknown";

/** App-level readiness shown under the brand mark. */
export type HostPresenceTone = "ready" | "unavailable" | "preview" | "connecting";

export type HostPresence = {
  tone: HostPresenceTone;
  /**
   * Primary label under Minos, e.g. "Online", "Connecting…", "Offline".
   * Never exposes managed / discovery / Host Link implementation detail.
   */
  label: string;
  readinessLabel: "Ready" | "Unavailable" | "Preview";
  cloud: CloudMode;
  cloudLabel: "Online" | "Connecting" | "Offline" | "—";
  /** True when local coding runtime is usable. */
  runtimeReady: boolean;
};

export type HostPresenceInput = {
  source: DataSource;
  /** True when Tauri bridge reports daemon connected. */
  daemonConnected: boolean;
  /**
   * Cloud connection (account signed in + hub).
   * Prefer store-driven status over raw hubOnline alone.
   */
  cloud?: CloudMode;
  /**
   * Live `/ws/host` when cloud status is not provided.
   * @deprecated Prefer `cloud`.
   */
  hubOnline?: boolean;
};

/** Default project locus for projects on this machine. */
export const PROJECT_HOST_THIS_MAC = "This Mac";

export function deriveHostPresence(input: HostPresenceInput): HostPresence {
  if (input.source === "mock") {
    return {
      tone: "preview",
      label: "Preview",
      readinessLabel: "Preview",
      cloud: "unknown",
      cloudLabel: "—",
      runtimeReady: false,
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
    };
  }

  const cloud: CloudMode =
    input.cloud ??
    (input.hubOnline === true
      ? "online"
      : input.hubOnline === false
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
    };
  }

  return {
    tone: "ready",
    label: "Ready",
    readinessLabel: "Ready",
    cloud: "unknown",
    cloudLabel: "—",
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
