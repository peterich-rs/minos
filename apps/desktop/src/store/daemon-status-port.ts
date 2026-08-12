/**
 * Narrow port for Host readiness flags without account↔workspace import cycles.
 *
 * Workspace connection registers the live refresh implementation at bootstrap.
 * Account ensure reads only this port (never dynamic-imports workspace-store).
 */

export type DaemonCloudFlags = {
  cloudOnline: boolean;
  hasHostToken: boolean;
};

type RefreshDaemonCloudFlags = () => Promise<DaemonCloudFlags>;

let refreshProvider: RefreshDaemonCloudFlags | null = null;

export function registerDaemonCloudFlagsProvider(
  next: RefreshDaemonCloudFlags,
): void {
  refreshProvider = next;
}

export async function refreshDaemonCloudFlags(): Promise<DaemonCloudFlags> {
  if (!refreshProvider) {
    return { cloudOnline: false, hasHostToken: false };
  }
  try {
    return await refreshProvider();
  } catch {
    return { cloudOnline: false, hasHostToken: false };
  }
}
